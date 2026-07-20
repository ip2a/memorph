use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::storage::{local_store, snapshot_store::ProjectedSessionSnapshotRow};

const DAY_MS: i64 = 86_400_000;
const LARGE_SESSION_BYTES: u64 = 10 * 1024 * 1024;
const SHORT_SESSION_MESSAGES: usize = 2;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DashboardRange {
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "90d")]
    NinetyDays,
    All,
}

impl Default for DashboardRange {
    fn default() -> Self {
        Self::ThirtyDays
    }
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
    pub message_count: usize,
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

pub fn dashboard(query: &StatsDashboardQuery) -> Result<StatsDashboard> {
    if !query.all && query.workspace.as_deref().is_none_or(str::is_empty) {
        anyhow::bail!("workspace is required when all=false");
    }
    dashboard_at(query, Utc::now())
}

fn dashboard_at(query: &StatsDashboardQuery, now: DateTime<Utc>) -> Result<StatsDashboard> {
    let conn = local_store::open_database()?;
    let rows =
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()?;
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|row| {
            query.all
                || crate::core::session_management::normalized_workspace_key(
                    &row.provider_id,
                    query.workspace.as_deref(),
                )
                .as_deref()
                    == row.workspace_dir.as_deref()
        })
        .collect();
    let created_at = load_created_at(&conn, &rows)?;
    let range_start = query.range.days().map(|days| now - Duration::days(days));
    Ok(build_dashboard(&rows, &created_at, range_start, now))
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
fn is_active(row: &ProjectedSessionSnapshotRow, start: Option<DateTime<Utc>>) -> bool {
    match start {
        Some(start) => row
            .last_active_at_ms
            .is_some_and(|ms| ms >= start.timestamp_millis()),
        None => row.last_active_at_ms.is_some(),
    }
}
fn add(bucket: &mut StatsBucket, row: &ProjectedSessionSnapshotRow) {
    bucket.count += 1;
    bucket.size_bytes += row.size_bytes.unwrap_or(0);
}

fn build_dashboard(
    rows: &[ProjectedSessionSnapshotRow],
    created: &HashMap<String, i64>,
    range_start: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> StatsDashboard {
    let active: Vec<_> = rows
        .iter()
        .filter(|row| is_active(row, range_start))
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
        total_messages: rows.iter().filter_map(|row| row.message_count).sum(),
        active_session_messages: active.iter().filter_map(|row| row.message_count).sum(),
        total_size_bytes: rows.iter().filter_map(|row| row.size_bytes).sum(),
        unknown_message_counts: rows
            .iter()
            .filter(|row| row.message_count.is_none())
            .count(),
        unknown_size_bytes: rows.iter().filter(|row| row.size_bytes.is_none()).count(),
        unknown_activity_times: rows
            .iter()
            .filter(|row| row.last_active_at_ms.is_none())
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
        match row
            .last_active_at_ms
            .map(|ms| (now.timestamp_millis() - ms).max(0) / DAY_MS)
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
        if row
            .message_count
            .is_some_and(|count| count <= SHORT_SESSION_MESSAGES)
        {
            add(&mut attention.short_sessions, row);
        }
    }

    let providers = breakdown(rows, &active, |row| Some(row.provider_id.clone()));
    let workspaces = breakdown(rows, &active, |row| row.workspace_dir.clone());
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
            message_count: row.message_count.unwrap_or_default(),
            size_bytes: row.size_bytes.unwrap_or_default(),
            created_at: created
                .get(&row.canonical_session_id)
                .and_then(|ms| date(*ms)),
            last_active_at: row.last_active_at_ms.and_then(date),
        })
        .collect();
    let mut by_messages = sessions.clone();
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
        timeline: timeline(rows, created, range_start, now),
        providers,
        workspaces,
        top_sessions: StatsTopSessions {
            by_messages,
            by_size,
            recently_active: sessions,
        },
        distributions: distributions(rows),
    }
}

fn breakdown<F>(
    rows: &[ProjectedSessionSnapshotRow],
    active: &[&ProjectedSessionSnapshotRow],
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
        item.message_count += row.message_count.unwrap_or(0);
        item.size_bytes += row.size_bytes.unwrap_or(0);
        let last = row.last_active_at_ms.and_then(date);
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
    for row in rows {
        if let Some(dt) = row
            .last_active_at_ms
            .and_then(date)
            .filter(|dt| *dt >= start)
        {
            let point = points
                .entry(key(dt))
                .or_insert_with(|| point_at(dt, monthly));
            point.active_sessions += 1;
            point.active_session_messages += row.message_count.unwrap_or(0);
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

fn distributions(rows: &[ProjectedSessionSnapshotRow]) -> StatsDistributions {
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
        let count = row.message_count.unwrap_or(0);
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
        let result = build_dashboard(&rows, &created, Some(now - Duration::days(30)), now);

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
        assert_eq!(result.attention.short_sessions.count, 2);
        assert_eq!(
            result
                .distributions
                .session_size
                .iter()
                .map(|bucket| bucket.count)
                .sum::<usize>(),
            4
        );
        assert_eq!(result.top_sessions.by_messages[0].session_id, "recent");
    }
}
