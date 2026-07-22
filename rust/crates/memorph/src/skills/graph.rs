use anyhow::{bail, Result};
use chrono::{Duration, Local, NaiveDate};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct GraphQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub skill_id: Option<String>,
    pub provider: Option<String>,
    pub workspace: Option<String>,
    pub timezone: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GraphDay {
    pub date: String,
    pub invocations: u64,
    pub sessions: u64,
    pub active_skills: u64,
    pub level: u8,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillGraph {
    pub from: String,
    pub to: String,
    pub timezone: String,
    pub max_count: u64,
    pub total_invocations: u64,
    pub days: Vec<GraphDay>,
}

pub fn graph(conn: &Connection, query: &GraphQuery) -> Result<SkillGraph> {
    let today = Local::now().date_naive();
    let to = query.to.as_deref().and_then(parse).unwrap_or(today);
    let from = query
        .from
        .as_deref()
        .and_then(parse)
        .unwrap_or(to - Duration::days(363));
    let timezone_modifier = timezone_modifier(query.timezone.as_deref())?;
    let mut statement = conn.prepare(
        "SELECT date(invoked_at_ms / 1000, 'unixepoch', ?6), COUNT(*), COUNT(DISTINCT session_id), COUNT(DISTINCT skill_id)
         FROM skill_invocations WHERE date(invoked_at_ms / 1000, 'unixepoch', ?6) BETWEEN ?1 AND ?2
           AND (?3 IS NULL OR skill_id = ?3) AND (?4 IS NULL OR provider_id = ?4)
           AND (?5 IS NULL OR workspace_dir = ?5) GROUP BY 1 ORDER BY 1",
    )?;
    let rows = statement.query_map(
        params![
            from.to_string(),
            to.to_string(),
            query.skill_id,
            query.provider,
            query.workspace,
            timezone_modifier,
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                ),
            ))
        },
    )?;
    let values = rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let mut counts = values
        .values()
        .map(|value| value.0)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    counts.sort_unstable();
    let level = |count: u64| -> u8 {
        if count == 0 || counts.is_empty() {
            return 0;
        }
        let position = counts.partition_point(|value| *value <= count);
        ((position * 4).div_ceil(counts.len())).clamp(1, 4) as u8
    };
    let mut days = Vec::new();
    let mut date = from;
    while date <= to {
        let key = date.to_string();
        let (invocations, sessions, active_skills) = values.get(&key).copied().unwrap_or_default();
        days.push(GraphDay {
            date: key,
            invocations,
            sessions,
            active_skills,
            level: level(invocations),
        });
        date += Duration::days(1);
    }
    Ok(SkillGraph {
        from: from.to_string(),
        to: to.to_string(),
        timezone: query.timezone.clone().unwrap_or_else(|| "local".into()),
        max_count: days.iter().map(|day| day.invocations).max().unwrap_or(0),
        total_invocations: days.iter().map(|day| day.invocations).sum(),
        days,
    })
}
fn parse(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn timezone_modifier(timezone: Option<&str>) -> Result<&str> {
    let Some(value) = timezone else {
        return Ok("localtime");
    };
    if value == "local" || value.contains('/') {
        return Ok("localtime");
    }
    let bytes = value.as_bytes();
    if bytes.len() != 6
        || !matches!(bytes[0], b'+' | b'-')
        || bytes[3] != b':'
        || !bytes[1..3].iter().all(u8::is_ascii_digit)
        || !bytes[4..6].iter().all(u8::is_ascii_digit)
    {
        bail!("invalid timezone offset: {value}");
    }
    let hours = (bytes[1] - b'0') * 10 + bytes[2] - b'0';
    let minutes = (bytes[4] - b'0') * 10 + bytes[5] - b'0';
    if hours > 14 || minutes > 59 || (hours == 14 && minutes != 0) {
        bail!("invalid timezone offset: {value}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fills_cross_year_and_leap_days_continuously() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE skill_invocations (invoked_at_ms INTEGER, session_id TEXT, skill_id TEXT, provider_id TEXT, workspace_dir TEXT);").unwrap();
        let result = graph(
            &conn,
            &GraphQuery {
                from: Some("2024-02-27".into()),
                to: Some("2024-03-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.days.len(), 4);
        assert_eq!(result.days[2].date, "2024-02-29");
    }

    #[test]
    fn applies_validated_timezone_offsets() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE skill_invocations (invoked_at_ms INTEGER, session_id TEXT, skill_id TEXT, provider_id TEXT, workspace_dir TEXT); INSERT INTO skill_invocations VALUES (1784656800000, 'session', 'skill', 'codex', '/work');").unwrap();
        let result = graph(
            &conn,
            &GraphQuery {
                from: Some("2026-07-22".into()),
                to: Some("2026-07-22".into()),
                timezone: Some("+08:00".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_invocations, 1);
        assert!(graph(
            &conn,
            &GraphQuery {
                timezone: Some("+15:00".into()),
                ..Default::default()
            }
        )
        .is_err());
    }
}
