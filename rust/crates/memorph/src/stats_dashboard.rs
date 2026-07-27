use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context as _, Result};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::storage::{local_store, snapshot_store::ProjectedSessionSnapshotRow};

const DAY_MS: i64 = 86_400_000;
const LARGE_SESSION_BYTES: u64 = 10 * 1024 * 1024;
const SHORT_SESSION_MESSAGES: usize = 2;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DashboardRange {
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    #[default]
    ThirtyDays,
    #[serde(rename = "90d")]
    NinetyDays,
    All,
}

impl DashboardRange {
    fn days(self) -> Option<i64> {
        match self {
            Self::SevenDays => Some(7),
            Self::ThirtyDays => Some(30),
            Self::NinetyDays => Some(90),
            Self::All => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StatsDashboardQuery {
    #[serde(default)]
    pub all: bool,
    pub workspace: Option<String>,
    #[serde(default)]
    pub range: DashboardRange,
}

#[derive(Debug, Serialize)]
pub struct StatsDashboard {
    pub generated_at: DateTime<Utc>,
    pub range_start: Option<DateTime<Utc>>,
    pub overview: StatsOverview,
    pub attention: StatsAttention,
    pub timeline: Vec<StatsTimelinePoint>,
    pub providers: Vec<StatsBreakdownItem>,
    pub workspaces: Vec<StatsBreakdownItem>,
    pub top_sessions: StatsTopSessions,
    pub distributions: StatsDistributions,
}

#[derive(Debug, Default, Serialize)]
pub struct StatsOverview {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub new_sessions: usize,
    pub total_messages: usize,
    pub active_session_messages: usize,
    pub total_size_bytes: u64,
    pub stale_size_bytes: u64,
    pub total_workspaces: usize,
    pub active_workspaces: usize,
    pub total_providers: usize,
    pub active_providers: usize,
    pub unknown_message_counts: usize,
    pub unknown_message_timestamps: usize,
    pub unknown_size_bytes: usize,
    pub unknown_activity_times: usize,
    pub unknown_created_times: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct StatsBucket {
    pub count: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StatsAttention {
    pub active_7d: StatsBucket,
    pub inactive_7_to_30d: StatsBucket,
    pub inactive_30_to_90d: StatsBucket,
    pub inactive_over_90d: StatsBucket,
    pub unknown: StatsBucket,
    pub large_sessions: StatsBucket,
    pub short_sessions: StatsBucket,
    pub large_threshold_bytes: u64,
    pub short_max_messages: usize,
}

#[derive(Debug, Serialize)]
pub struct StatsTimelinePoint {
    pub start: DateTime<Utc>,
    pub active_sessions: usize,
    pub new_sessions: usize,
    pub active_session_messages: usize,
    pub new_size_bytes: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct StatsBreakdownItem {
    pub id: String,
    pub session_count: usize,
    pub active_session_count: usize,
    pub message_count: usize,
    pub size_bytes: u64,
    pub last_active_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSessionItem {
    pub provider_id: String,
    pub session_id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub message_count: Option<usize>,
    pub size_bytes: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub last_active_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct StatsTopSessions {
    pub by_messages: Vec<StatsSessionItem>,
    pub by_size: Vec<StatsSessionItem>,
    pub recently_active: Vec<StatsSessionItem>,
}

#[derive(Debug, Serialize)]
pub struct StatsDistributionBucket {
    pub key: &'static str,
    pub label: &'static str,
    pub count: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StatsDistributions {
    pub session_size: Vec<StatsDistributionBucket>,
    pub message_count: Vec<StatsDistributionBucket>,
}

#[derive(Clone, Copy)]
struct DailyEventFacts {
    day_start_ms: i64,
    event_count: usize,
    message_count: usize,
}

#[derive(Default)]
struct EventFacts {
    message_count: usize,
    last_activity_at_ms: Option<i64>,
    daily: Vec<DailyEventFacts>,
    timestamped_messages: usize,
}

fn snapshot_facts(rows: &[ProjectedSessionSnapshotRow]) -> HashMap<String, EventFacts> {
    rows.iter()
        .filter_map(|row| {
            row.message_count.map(|message_count| {
                (
                    row.canonical_session_id.clone(),
                    EventFacts {
                        message_count,
                        last_activity_at_ms: row.last_active_at_ms,
                        daily: Vec::new(),
                        timestamped_messages: 0,
                    },
                )
            })
        })
        .collect()
}

pub fn dashboard(query: &StatsDashboardQuery) -> Result<StatsDashboard> {
    if !query.all && query.workspace.as_deref().is_none_or(str::is_empty) {
        anyhow::bail!("workspace is required when all=false");
    }
    dashboard_at(query, Utc::now())
}

fn dashboard_at(query: &StatsDashboardQuery, now: DateTime<Utc>) -> Result<StatsDashboard> {
    let mut conn = local_store::open_database()?;
    let filter_rows = |rows: Vec<ProjectedSessionSnapshotRow>| {
        rows.into_iter()
            .filter(|row| {
                query.all
                    || crate::core::session_management::normalized_workspace_key(
                        &row.provider_id,
                        query.workspace.as_deref(),
                    )
                    .as_deref()
                        == row.workspace_dir.as_deref()
            })
            .collect::<Vec<_>>()
    };
    let rows = filter_rows(
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()?,
    );
    complete_missing_counts(&mut conn, &rows)?;
    let rows = filter_rows(
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()?,
    );
    let created_at = load_created_at(&conn, &rows)?;
    let event_facts = load_event_facts(&conn, &rows)?;
    let range_start = query.range.days().map(|days| now - Duration::days(days));
    Ok(build_dashboard(
        &rows,
        &created_at,
        &event_facts,
        range_start,
        now,
    ))
}

struct MissingCountComputation {
    canonical_session_id: String,
    provider_id: String,
    provider_session_id: Option<String>,
    title: Option<String>,
    workspace_dir: Option<String>,
    last_active_at_ms: Option<i64>,
    source_path: String,
    capabilities: crate::provider::ProviderCapabilities,
    fingerprint: crate::provider::ProviderSourceFingerprint,
    event_count: usize,
    message_count: usize,
    turn_count: usize,
    events: Vec<crate::canonical::Event>,
}

fn compute_missing_count(row: &ProjectedSessionSnapshotRow) -> Result<MissingCountComputation> {
    let source_path = row
        .source_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| {
            format!(
                "Cannot count messages for {}/{}: source path is missing",
                row.provider_id, row.canonical_session_id
            )
        })?;
    let provider = crate::providers::find_provider(&row.provider_id)
        .with_context(|| format!("Unknown provider: {}", row.provider_id))?;
    let capabilities = provider.capabilities();
    if !capabilities.import {
        anyhow::bail!(
            "Provider does not support message counting: {}",
            row.provider_id
        );
    }
    let fingerprint = provider
        .session_source_fingerprint(source_path)?
        .with_context(|| format!("Session source is missing: {source_path}"))?;
    let page = provider
        .import_session_page(source_path, 0, None)
        .with_context(|| {
            format!(
                "Failed to count messages for {}/{}",
                row.provider_id, row.canonical_session_id
            )
        })?;
    let turn_count = page.turn_count.unwrap_or_else(|| {
        crate::session_projection::project_session_turns(
            &row.canonical_session_id,
            &page.imported.session.events,
            capabilities.turn_quality,
        )
        .len()
    });
    Ok(MissingCountComputation {
        canonical_session_id: row.canonical_session_id.clone(),
        provider_id: row.provider_id.clone(),
        provider_session_id: row.provider_session_id.clone(),
        title: row.title.clone(),
        workspace_dir: row.workspace_dir.clone(),
        last_active_at_ms: row.last_active_at_ms,
        source_path: source_path.to_string(),
        capabilities,
        fingerprint,
        event_count: page.event_count,
        message_count: page.message_count,
        turn_count,
        events: page.imported.session.events,
    })
}

fn complete_missing_counts(
    conn: &mut rusqlite::Connection,
    rows: &[ProjectedSessionSnapshotRow],
) -> Result<()> {
    let missing: Vec<_> = rows
        .iter()
        .filter(|row| row.message_count.is_none())
        .collect();
    let worker_count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(missing.len().max(1));
    let chunk_size = missing.len().div_ceil(worker_count).max(1);
    let computations = std::thread::scope(|scope| {
        missing
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|row| compute_missing_count(row))
                        .collect::<Result<Vec<_>>>()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("message count worker panicked"))?
            })
            .collect::<Result<Vec<_>>>()
            .map(|chunks| chunks.into_iter().flatten().collect::<Vec<_>>())
    })?;

    let mut store = crate::storage::session_index_store::SessionIndexStore::new(conn);
    for computation in computations {
        let mut canonical_session_id = computation.canonical_session_id.clone();
        if !store.record_complete_counts(
            &canonical_session_id,
            &computation.fingerprint.value,
            computation.event_count,
            computation.message_count,
            computation.turn_count,
        )? {
            let indexed = store.write_session_summary(
                &computation.provider_id,
                &crate::provider::ProviderSessionSummary {
                    session_id: computation
                        .provider_session_id
                        .clone()
                        .unwrap_or_else(|| computation.canonical_session_id.clone()),
                    title: computation.title.clone(),
                    project_dir: computation.workspace_dir.clone(),
                    created_at: None,
                    last_active_at: computation.last_active_at_ms,
                    source_path: Some(computation.source_path.clone()),
                },
                computation.capabilities,
                &computation.fingerprint,
            )?;
            canonical_session_id = indexed.canonical_session_id;
            if !store.record_complete_counts(
                &canonical_session_id,
                &computation.fingerprint.value,
                computation.event_count,
                computation.message_count,
                computation.turn_count,
            )? {
                anyhow::bail!(
                    "Session changed while counting messages: {}/{}",
                    computation.provider_id,
                    computation.canonical_session_id
                );
            }
        }
        store.replace_daily_stats(&canonical_session_id, &computation.events)?;
    }
    Ok(())
}

fn load_event_facts(
    conn: &rusqlite::Connection,
    rows: &[ProjectedSessionSnapshotRow],
) -> Result<HashMap<String, EventFacts>> {
    let mut facts = snapshot_facts(rows);
    if rows.is_empty() {
        return Ok(facts);
    }
    let ids: HashSet<_> = rows
        .iter()
        .map(|row| row.canonical_session_id.as_str())
        .collect();
    let mut stmt = conn.prepare(
        "SELECT session_id, day_start_ms, event_count, message_count FROM session_daily_stats",
    )?;
    for row in stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?.max(0) as usize,
            row.get::<_, i64>(3)?.max(0) as usize,
        ))
    })? {
        let (session_id, day, event_count, message_count) = row?;
        if !ids.contains(session_id.as_str()) {
            continue;
        }
        if let Some(item) = facts.get_mut(&session_id) {
            item.daily.push(DailyEventFacts {
                day_start_ms: day,
                event_count,
                message_count,
            });
            item.timestamped_messages += message_count;
        }
    }
    Ok(facts)
}

fn load_created_at(
    conn: &rusqlite::Connection,
    rows: &[ProjectedSessionSnapshotRow],
) -> Result<HashMap<String, i64>> {
    if rows.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: HashSet<_> = rows
        .iter()
        .map(|row| row.canonical_session_id.as_str())
        .collect();
    let mut stmt =
        conn.prepare("SELECT id, created_at_ms FROM sessions WHERE deleted_at_ms IS NULL")?;
    let values = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
        })?
        .filter_map(|row| row.ok())
        .filter_map(|(id, value)| value.map(|value| (id, value)))
        .filter(|(id, _)| ids.contains(id.as_str()))
        .collect();
    Ok(values)
}

fn date(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}
fn activity_at(
    row: &ProjectedSessionSnapshotRow,
    event_facts: &HashMap<String, EventFacts>,
) -> Option<i64> {
    event_facts
        .get(&row.canonical_session_id)
        .and_then(|facts| facts.last_activity_at_ms)
        .or(row.last_active_at_ms)
}

fn is_active(
    row: &ProjectedSessionSnapshotRow,
    event_facts: &HashMap<String, EventFacts>,
    start: Option<DateTime<Utc>>,
) -> bool {
    match activity_at(row, event_facts) {
        Some(ms) => start.is_none_or(|start| ms >= start.timestamp_millis()),
        None => false,
    }
}
fn add(bucket: &mut StatsBucket, row: &ProjectedSessionSnapshotRow) {
    bucket.count += 1;
    bucket.size_bytes += row.size_bytes.unwrap_or(0);
}

fn build_dashboard(
    rows: &[ProjectedSessionSnapshotRow],
    created: &HashMap<String, i64>,
    event_facts: &HashMap<String, EventFacts>,
    range_start: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> StatsDashboard {
    let active: Vec<_> = rows
        .iter()
        .filter(|row| is_active(row, event_facts, range_start))
        .collect();
    let mut overview = StatsOverview {
        total_sessions: rows.len(),
        active_sessions: active.len(),
        new_sessions: rows
            .iter()
            .filter(|row| {
                created.get(&row.canonical_session_id).is_some_and(|ms| {
                    range_start.is_none_or(|start| *ms >= start.timestamp_millis())
                })
            })
            .count(),
        total_messages: rows
            .iter()
            .map(|row| {
                event_facts
                    .get(&row.canonical_session_id)
                    .map_or(0, |facts| facts.message_count)
            })
            .sum(),
        active_session_messages: event_facts
            .values()
            .flat_map(|facts| &facts.daily)
            .filter(|daily| {
                range_start.is_none_or(|start| daily.day_start_ms >= start.timestamp_millis())
            })
            .map(|daily| daily.message_count)
            .sum(),
        total_size_bytes: rows.iter().filter_map(|row| row.size_bytes).sum(),
        unknown_message_counts: rows
            .iter()
            .filter(|row| !event_facts.contains_key(&row.canonical_session_id))
            .count(),
        unknown_message_timestamps: event_facts
            .values()
            .map(|facts| {
                facts
                    .message_count
                    .saturating_sub(facts.timestamped_messages)
            })
            .sum(),
        unknown_size_bytes: rows.iter().filter(|row| row.size_bytes.is_none()).count(),
        unknown_activity_times: rows
            .iter()
            .filter(|row| activity_at(row, event_facts).is_none())
            .count(),
        unknown_created_times: rows
            .iter()
            .filter(|row| !created.contains_key(&row.canonical_session_id))
            .count(),
        ..Default::default()
    };
    overview.stale_size_bytes = rows
        .iter()
        .filter(|row| {
            row.last_active_at_ms
                .is_some_and(|ms| ms < now.timestamp_millis() - 90 * DAY_MS)
        })
        .filter_map(|row| row.size_bytes)
        .sum();
    overview.total_workspaces = rows
        .iter()
        .filter_map(|row| row.workspace_dir.as_deref())
        .collect::<HashSet<_>>()
        .len();
    overview.active_workspaces = active
        .iter()
        .filter_map(|row| row.workspace_dir.as_deref())
        .collect::<HashSet<_>>()
        .len();
    overview.total_providers = rows
        .iter()
        .map(|row| row.provider_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    overview.active_providers = active
        .iter()
        .map(|row| row.provider_id.as_str())
        .collect::<HashSet<_>>()
        .len();

    let mut attention = StatsAttention {
        active_7d: Default::default(),
        inactive_7_to_30d: Default::default(),
        inactive_30_to_90d: Default::default(),
        inactive_over_90d: Default::default(),
        unknown: Default::default(),
        large_sessions: Default::default(),
        short_sessions: Default::default(),
        large_threshold_bytes: LARGE_SESSION_BYTES,
        short_max_messages: SHORT_SESSION_MESSAGES,
    };
    for row in rows {
        match activity_at(row, event_facts).map(|ms| (now.timestamp_millis() - ms).max(0) / DAY_MS)
        {
            None => add(&mut attention.unknown, row),
            Some(days) if days < 7 => add(&mut attention.active_7d, row),
            Some(days) if days < 30 => add(&mut attention.inactive_7_to_30d, row),
            Some(days) if days < 90 => add(&mut attention.inactive_30_to_90d, row),
            Some(_) => add(&mut attention.inactive_over_90d, row),
        }
        if row.size_bytes.unwrap_or(0) >= LARGE_SESSION_BYTES {
            add(&mut attention.large_sessions, row);
        }
        if event_facts
            .get(&row.canonical_session_id)
            .is_some_and(|facts| facts.message_count <= SHORT_SESSION_MESSAGES)
        {
            add(&mut attention.short_sessions, row);
        }
    }

    let providers = breakdown(rows, &active, event_facts, |row| {
        Some(row.provider_id.clone())
    });
    let workspaces = breakdown(rows, &active, event_facts, |row| row.workspace_dir.clone());
    let mut sessions: Vec<_> = rows
        .iter()
        .map(|row| StatsSessionItem {
            provider_id: row.provider_id.clone(),
            session_id: row
                .provider_session_id
                .clone()
                .unwrap_or_else(|| row.canonical_session_id.clone()),
            title: row
                .display_title
                .clone()
                .or_else(|| row.title.clone())
                .unwrap_or_else(|| "Untitled session".into()),
            workspace: row.workspace_dir.clone(),
            message_count: event_facts
                .get(&row.canonical_session_id)
                .map(|facts| facts.message_count),
            size_bytes: row.size_bytes.unwrap_or_default(),
            created_at: created
                .get(&row.canonical_session_id)
                .and_then(|ms| date(*ms)),
            last_active_at: activity_at(row, event_facts).and_then(date),
        })
        .collect();
    let mut by_messages: Vec<_> = sessions
        .iter()
        .filter(|item| item.message_count.is_some())
        .cloned()
        .collect();
    by_messages.sort_by_key(|item| std::cmp::Reverse(item.message_count));
    by_messages.truncate(10);
    let mut by_size = sessions.clone();
    by_size.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    by_size.truncate(10);
    sessions.sort_by_key(|item| std::cmp::Reverse(item.last_active_at));
    sessions.truncate(10);

    StatsDashboard {
        generated_at: now,
        range_start,
        overview,
        attention,
        timeline: timeline(rows, created, event_facts, range_start, now),
        providers,
        workspaces,
        top_sessions: StatsTopSessions {
            by_messages,
            by_size,
            recently_active: sessions,
        },
        distributions: distributions(rows, event_facts),
    }
}

fn breakdown<F>(
    rows: &[ProjectedSessionSnapshotRow],
    active: &[&ProjectedSessionSnapshotRow],
    event_facts: &HashMap<String, EventFacts>,
    key: F,
) -> Vec<StatsBreakdownItem>
where
    F: Fn(&ProjectedSessionSnapshotRow) -> Option<String>,
{
    let active_ids: HashSet<_> = active
        .iter()
        .map(|row| row.canonical_session_id.as_str())
        .collect();
    let mut values: HashMap<String, StatsBreakdownItem> = HashMap::new();
    for row in rows {
        let Some(id) = key(row) else { continue };
        let item = values
            .entry(id.clone())
            .or_insert_with(|| StatsBreakdownItem {
                id,
                ..Default::default()
            });
        item.session_count += 1;
        item.active_session_count +=
            usize::from(active_ids.contains(row.canonical_session_id.as_str()));
        item.message_count += event_facts
            .get(&row.canonical_session_id)
            .map_or(0, |facts| facts.message_count);
        item.size_bytes += row.size_bytes.unwrap_or(0);
        let last = activity_at(row, event_facts).and_then(date);
        if last > item.last_active_at {
            item.last_active_at = last;
        }
    }
    let mut values: Vec<_> = values.into_values().collect();
    values.sort_by_key(|item| std::cmp::Reverse(item.session_count));
    values
}

fn timeline(
    rows: &[ProjectedSessionSnapshotRow],
    created: &HashMap<String, i64>,
    event_facts: &HashMap<String, EventFacts>,
    range_start: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<StatsTimelinePoint> {
    let start = range_start
        .or_else(|| {
            rows.iter()
                .filter_map(|row| {
                    created
                        .get(&row.canonical_session_id)
                        .copied()
                        .or(row.last_active_at_ms)
                })
                .min()
                .and_then(date)
        })
        .unwrap_or(now);
    let monthly = (now - start).num_days() > 180;
    let mut points: BTreeMap<(i32, u32, u32), StatsTimelinePoint> = BTreeMap::new();
    let key = |dt: DateTime<Utc>| {
        if monthly {
            (dt.year(), dt.month(), 1)
        } else {
            (dt.year(), dt.month(), dt.day())
        }
    };
    let mut cursor = point_at(start, monthly).start;
    loop {
        points.insert(key(cursor), point_at(cursor, monthly));
        if key(cursor) >= key(now) {
            break;
        }
        cursor = if monthly {
            let (year, month) = if cursor.month() == 12 {
                (cursor.year() + 1, 1)
            } else {
                (cursor.year(), cursor.month() + 1)
            };
            Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).unwrap()
        } else {
            cursor + Duration::days(1)
        };
    }
    for row in rows {
        if let Some(facts) = event_facts.get(&row.canonical_session_id) {
            if facts.daily.is_empty() {
                if let Some(dt) = facts
                    .last_activity_at_ms
                    .or(row.last_active_at_ms)
                    .and_then(date)
                    .filter(|dt| *dt >= start)
                {
                    let point = points
                        .entry(key(dt))
                        .or_insert_with(|| point_at(dt, monthly));
                    point.active_sessions += 1;
                }
            } else {
                let mut active_periods: HashMap<_, (DateTime<Utc>, usize)> = HashMap::new();
                for daily in &facts.daily {
                    if daily.event_count == 0 {
                        continue;
                    }
                    if let Some(dt) = date(daily.day_start_ms).filter(|dt| *dt >= start) {
                        let entry = active_periods.entry(key(dt)).or_insert((dt, 0));
                        entry.1 += daily.message_count;
                    }
                }
                for (bucket_key, (dt, message_count)) in active_periods {
                    let point = points
                        .entry(bucket_key)
                        .or_insert_with(|| point_at(dt, monthly));
                    point.active_sessions += 1;
                    point.active_session_messages += message_count;
                }
            }
        } else if let Some(dt) = row
            .last_active_at_ms
            .and_then(date)
            .filter(|dt| *dt >= start)
        {
            let point = points
                .entry(key(dt))
                .or_insert_with(|| point_at(dt, monthly));
            point.active_sessions += 1;
        }
        if let Some(dt) = created
            .get(&row.canonical_session_id)
            .and_then(|ms| date(*ms))
            .filter(|dt| *dt >= start)
        {
            let point = points
                .entry(key(dt))
                .or_insert_with(|| point_at(dt, monthly));
            point.new_sessions += 1;
            point.new_size_bytes += row.size_bytes.unwrap_or(0);
        }
    }
    points.into_values().collect()
}

fn point_at(dt: DateTime<Utc>, monthly: bool) -> StatsTimelinePoint {
    let day = if monthly { 1 } else { dt.day() };
    let start = Utc
        .with_ymd_and_hms(dt.year(), dt.month(), day, 0, 0, 0)
        .unwrap();
    StatsTimelinePoint {
        start,
        active_sessions: 0,
        new_sessions: 0,
        active_session_messages: 0,
        new_size_bytes: 0,
    }
}

fn distributions(
    rows: &[ProjectedSessionSnapshotRow],
    event_facts: &HashMap<String, EventFacts>,
) -> StatsDistributions {
    let mut sizes = [
        ("lt_100kb", "小于 100 KB", 0, 0),
        ("100kb_1mb", "100 KB–1 MB", 0, 0),
        ("1mb_10mb", "1–10 MB", 0, 0),
        ("gte_10mb", "10 MB 以上", 0, 0),
    ];
    let mut messages = [
        ("0_2", "0–2 条", 0, 0),
        ("3_20", "3–20 条", 0, 0),
        ("21_100", "21–100 条", 0, 0),
        ("gt_100", "100 条以上", 0, 0),
    ];
    for row in rows {
        let Some(facts) = event_facts.get(&row.canonical_session_id) else {
            continue;
        };
        let size = row.size_bytes.unwrap_or(0);
        let si = if size < 100 * 1024 {
            0
        } else if size < 1024 * 1024 {
            1
        } else if size < 10 * 1024 * 1024 {
            2
        } else {
            3
        };
        sizes[si].2 += 1;
        sizes[si].3 += size;
        let count = facts.message_count;
        let mi = if count <= 2 {
            0
        } else if count <= 20 {
            1
        } else if count <= 100 {
            2
        } else {
            3
        };
        messages[mi].2 += 1;
        messages[mi].3 += size;
    }
    let map = |(key, label, count, size_bytes)| StatsDistributionBucket {
        key,
        label,
        count,
        size_bytes,
    };
    StatsDistributions {
        session_size: sizes.into_iter().map(map).collect(),
        message_count: messages.into_iter().map(map).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: &str,
        provider: &str,
        workspace: Option<&str>,
        last_active_at_ms: Option<i64>,
        messages: usize,
        size: u64,
    ) -> ProjectedSessionSnapshotRow {
        ProjectedSessionSnapshotRow {
            canonical_session_id: id.into(),
            provider_id: provider.into(),
            provider_session_id: Some(id.into()),
            title: Some(id.into()),
            display_title: None,
            workspace_dir: workspace.map(str::to_string),
            last_active_at_ms,
            source_path: None,
            message_count: Some(messages),
            event_count: messages,
            turn_count: 0,
            size_bytes: Some(size),
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        }
    }

    #[test]
    fn loads_message_counts_from_bodyless_snapshots() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_daily_stats (
                session_id TEXT, day_start_ms INTEGER, event_count INTEGER, message_count INTEGER
            );",
        )
        .unwrap();
        let rows = vec![row("session", "codex", Some("/a"), Some(20), 2, 100)];

        let facts = load_event_facts(&conn, &rows).unwrap();
        let session = &facts["session"];

        assert_eq!(session.message_count, 2);
        assert_eq!(session.last_activity_at_ms, Some(20));
        assert!(session.daily.is_empty());
        assert_eq!(session.timestamped_messages, 0);
    }

    #[test]
    fn timeline_falls_back_to_last_active_when_timestamps_missing() {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
        let active_ms = (now - Duration::days(3)).timestamp_millis();
        let rows = vec![
            row(
                "with-messages",
                "codex",
                Some("/a"),
                Some(active_ms),
                12,
                100,
            ),
            row("no-messages", "claude", Some("/a"), Some(active_ms), 0, 50),
        ];
        // Simulate production snapshot_facts: message count present, daily facts missing.
        let events = HashMap::from([(
            "with-messages".into(),
            EventFacts {
                message_count: 12,
                last_activity_at_ms: Some(active_ms),
                daily: Vec::new(),
                timestamped_messages: 0,
            },
        )]);
        let result = build_dashboard(
            &rows,
            &HashMap::new(),
            &events,
            Some(now - Duration::days(7)),
            now,
        );

        assert_eq!(result.timeline.len(), 8);
        assert_eq!(
            result
                .timeline
                .iter()
                .map(|point| point.active_sessions)
                .sum::<usize>(),
            2
        );
        assert_eq!(
            result
                .timeline
                .iter()
                .map(|point| point.new_sessions)
                .sum::<usize>(),
            0
        );
    }

    #[test]
    fn uses_event_timestamps_for_activity_and_message_counts() {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
        let rows = vec![row("session", "codex", Some("/a"), None, 99, 100)];
        let events = HashMap::from([(
            "session".into(),
            EventFacts {
                message_count: 3,
                last_activity_at_ms: Some((now - Duration::days(1)).timestamp_millis()),
                daily: vec![
                    DailyEventFacts {
                        day_start_ms: (now - Duration::days(2)).timestamp_millis(),
                        event_count: 2,
                        message_count: 2,
                    },
                    DailyEventFacts {
                        day_start_ms: (now - Duration::days(1)).timestamp_millis(),
                        event_count: 1,
                        message_count: 1,
                    },
                ],
                timestamped_messages: 3,
            },
        )]);
        let result = build_dashboard(
            &rows,
            &HashMap::new(),
            &events,
            Some(now - Duration::days(7)),
            now,
        );

        assert_eq!(result.overview.total_messages, 3);
        assert_eq!(result.overview.active_sessions, 1);
        assert_eq!(result.overview.active_session_messages, 3);
        assert_eq!(result.top_sessions.by_messages[0].message_count, Some(3));
        assert_eq!(
            result.top_sessions.recently_active[0].last_active_at,
            date((now - Duration::days(1)).timestamp_millis())
        );
    }

    #[test]
    fn builds_macro_attention_and_distribution_statistics() {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
        let rows = vec![
            row(
                "recent",
                "codex",
                Some("/a"),
                Some((now - Duration::days(2)).timestamp_millis()),
                50,
                12 * 1024 * 1024,
            ),
            row(
                "month",
                "claude",
                Some("/a"),
                Some((now - Duration::days(45)).timestamp_millis()),
                20,
                2 * 1024 * 1024,
            ),
            row(
                "old",
                "claude",
                Some("/b"),
                Some((now - Duration::days(120)).timestamp_millis()),
                1,
                512,
            ),
            row("unknown", "cursor", None, None, 0, 128),
        ];
        let created = HashMap::from([
            (
                "recent".into(),
                (now - Duration::days(1)).timestamp_millis(),
            ),
            (
                "month".into(),
                (now - Duration::days(60)).timestamp_millis(),
            ),
        ]);
        let event_facts = HashMap::from([
            (
                "recent".into(),
                EventFacts {
                    message_count: 50,
                    ..Default::default()
                },
            ),
            (
                "month".into(),
                EventFacts {
                    message_count: 20,
                    ..Default::default()
                },
            ),
            (
                "old".into(),
                EventFacts {
                    message_count: 1,
                    ..Default::default()
                },
            ),
        ]);
        let result = build_dashboard(
            &rows,
            &created,
            &event_facts,
            Some(now - Duration::days(30)),
            now,
        );

        assert_eq!(result.overview.total_sessions, 4);
        assert_eq!(result.overview.active_sessions, 1);
        assert_eq!(result.overview.new_sessions, 1);
        assert_eq!(result.overview.total_workspaces, 2);
        assert_eq!(result.overview.total_providers, 3);
        assert_eq!(result.attention.active_7d.count, 1);
        assert_eq!(result.attention.inactive_30_to_90d.count, 1);
        assert_eq!(result.attention.inactive_over_90d.count, 1);
        assert_eq!(result.attention.unknown.count, 1);
        assert_eq!(result.attention.large_sessions.count, 1);
        assert_eq!(result.attention.short_sessions.count, 1);
        assert_eq!(
            result
                .distributions
                .session_size
                .iter()
                .map(|bucket| bucket.count)
                .sum::<usize>(),
            3
        );
        assert_eq!(result.top_sessions.by_messages[0].session_id, "recent");
        assert!(result
            .top_sessions
            .by_messages
            .iter()
            .all(|item| item.session_id != "unknown"));
        assert_eq!(
            result
                .top_sessions
                .by_size
                .iter()
                .find(|item| item.session_id == "unknown")
                .and_then(|item| item.message_count),
            None
        );
    }
}
