use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::repository::{self, SessionSourceRecord};
use crate::{
    providers,
    session::{Block, Event},
};

#[derive(Clone, Debug)]
struct SkillIdentity {
    id: String,
    fingerprint: String,
    names: BTreeSet<String>,
    installations: Vec<(String, String)>,
    files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Invocation {
    skill_id: String,
    installation_id: Option<String>,
    event_id: String,
    invoked_at_ms: i64,
    detection_kind: &'static str,
    confidence: &'static str,
    evidence_text: String,
    evidence_path: Option<String>,
    token_count: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct IndexSummary {
    pub sources_scanned: usize,
    pub sources_skipped: usize,
    pub sources_failed: usize,
    pub sessions_scanned: usize,
    pub invocations_indexed: usize,
    pub earliest_indexed_at_ms: Option<i64>,
    pub latest_indexed_at_ms: Option<i64>,
    pub completeness_status: String,
}

#[derive(Clone, Debug, Default)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub provider: Option<String>,
    pub workspace: Option<String>,
    pub confidence: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StatsSummary {
    pub invocations: u64,
    pub active_skills: u64,
    pub active_sessions: u64,
    pub active_days: u64,
    pub token_count: Option<u64>,
    pub last_invoked_at_ms: Option<i64>,
    pub completeness_status: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DailyUsage {
    pub date: String,
    pub invocations: u64,
    pub sessions: u64,
    pub token_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillRanking {
    pub skill_id: String,
    pub name: String,
    pub invocations: u64,
    pub sessions: u64,
    pub token_count: Option<u64>,
    pub last_invoked_at_ms: Option<i64>,
    pub trend: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StatsBreakdownItem {
    pub key: String,
    pub invocations: u64,
    pub sessions: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StatsBreakdown {
    pub providers: Vec<StatsBreakdownItem>,
    pub workspaces: Vec<StatsBreakdownItem>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InvocationItem {
    pub id: String,
    pub session_id: String,
    pub event_id: Option<String>,
    pub provider_id: String,
    pub workspace_dir: Option<String>,
    pub invoked_at_ms: i64,
    pub detection_kind: String,
    pub confidence: String,
    pub evidence_text: Option<String>,
    pub evidence_path: Option<String>,
    pub token_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InvocationPage {
    pub items: Vec<InvocationItem>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
}

pub fn index(conn: &mut Connection, force: bool) -> Result<IndexSummary> {
    index_impl(conn, force, false)
}

pub fn index_sources_incrementally(conn: &mut Connection) -> Result<IndexSummary> {
    index_impl(conn, false, true)
}

fn index_impl(
    conn: &mut Connection,
    force: bool,
    invalidate_catalog: bool,
) -> Result<IndexSummary> {
    let now_ms = Utc::now().timestamp_millis();
    let identities = load_identities(conn)?;
    let catalog_generation = catalog_generation_from_identities(&identities);
    let sources = repository::session_sources(conn)?;
    let aggregate_key = "skill-sessions:all";
    if invalidate_catalog {
        conn.execute(
            "UPDATE skill_scan_state SET completeness_status = 'partial', updated_at_ms = ?2
             WHERE state_kind = 'session-source'
               AND COALESCE(json_extract(details_json, '$.catalog_generation'), '') <> ?1",
            params![catalog_generation, now_ms],
        )?;
    }
    repository::begin_scan(conn, aggregate_key, "aggregate", None, None, now_ms)?;
    let mut summary = IndexSummary::default();

    for source in &sources {
        let state_key = format!("session-source:{}", source.id);
        if !force
            && repository::session_source_scan_is_current(
                conn,
                &state_key,
                &source.fingerprint,
                source.source_cursor.as_deref(),
                &catalog_generation,
            )?
        {
            summary.sources_skipped += 1;
            continue;
        }
        repository::begin_scan(
            conn,
            &state_key,
            "session-source",
            Some(&source.provider_id),
            Some(&source.source_path),
            now_ms,
        )?;
        match index_source(conn, source, &identities, now_ms) {
            Ok(count) => {
                summary.sources_scanned += 1;
                summary.sessions_scanned += usize::from(source.session_id.is_some());
                summary.invocations_indexed += count;
                repository::complete_scan(
                    conn,
                    &state_key,
                    Some(&source.fingerprint),
                    source.source_cursor.as_deref(),
                    count,
                    false,
                    "complete",
                    source.earliest_at_ms,
                    source.latest_at_ms,
                    now_ms,
                )?;
                repository::set_scan_catalog_generation(conn, &state_key, &catalog_generation)?;
            }
            Err(error) => {
                summary.sources_failed += 1;
                repository::fail_scan(conn, &state_key, &format!("{error:#}"), now_ms)?;
            }
        }
    }
    if force || summary.sources_scanned > 0 {
        rebuild_daily(conn, now_ms)?;
        super::coverage::rebuild(conn)?;
    }
    let (count, earliest, latest): (i64, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT COUNT(*), MIN(invoked_at_ms), MAX(invoked_at_ms) FROM skill_invocations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    summary.invocations_indexed = count as usize;
    summary.earliest_indexed_at_ms = earliest;
    summary.latest_indexed_at_ms = latest;
    summary.completeness_status = if summary.sources_failed == 0 {
        "complete"
    } else {
        "partial"
    }
    .into();
    repository::complete_scan(
        conn,
        aggregate_key,
        None,
        None,
        sources.len(),
        false,
        &summary.completeness_status,
        earliest,
        latest,
        now_ms,
    )?;
    repository::set_scan_catalog_generation(conn, aggregate_key, &catalog_generation)?;
    Ok(summary)
}

fn index_source(
    conn: &mut Connection,
    source: &SessionSourceRecord,
    identities: &[SkillIdentity],
    now_ms: i64,
) -> Result<usize> {
    let Some(session_id) = source.session_id.as_deref() else {
        return Ok(0);
    };
    let provider = providers::find_provider(&source.provider_id)
        .with_context(|| format!("Unknown provider: {}", source.provider_id))?;
    let imported = provider.import_session_page(&source.source_path, 0, None)?;
    let invocations = imported
        .imported
        .session
        .events
        .iter()
        .flat_map(|event| detect_event(event, identities))
        .collect::<Vec<_>>();
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM skill_invocations WHERE source_id = ?1",
        [&source.id],
    )?;
    for item in &invocations {
        let id = hash(format!("{}:{}:{}", source.id, item.event_id, item.skill_id).as_bytes());
        tx.execute(
            "INSERT INTO skill_invocations
             (id, skill_id, installation_id, session_id, source_id, event_id, provider_id,
              workspace_dir, invoked_at_ms, detection_kind, confidence, evidence_text,
              evidence_path, token_count, source_fingerprint, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)",
            params![
                id,
                item.skill_id,
                item.installation_id,
                session_id,
                source.id,
                item.event_id,
                source.provider_id,
                source.workspace_dir,
                item.invoked_at_ms,
                item.detection_kind,
                item.confidence,
                item.evidence_text,
                item.evidence_path,
                item.token_count.map(|v| v as i64),
                source.fingerprint,
                now_ms
            ],
        )?;
    }
    tx.commit()?;
    Ok(invocations.len())
}

fn detect_event(event: &Event, identities: &[SkillIdentity]) -> Vec<Invocation> {
    let text = event_text(event);
    let normalized_text = text.replace('\\', "/").to_lowercase();
    let explicit = explicit_names(event);
    let mut matches = BTreeMap::<String, (u8, Invocation)>::new();
    for skill in identities {
        let mut candidate = None;
        if let Some(name) = explicit.iter().find(|name| skill.names.contains(*name)) {
            candidate = Some((5, None, "explicit-tool", "high", name.clone(), None));
        }
        for (installation_id, path) in &skill.installations {
            let entry = format!("{}/skill.md", path.replace('\\', "/").trim_end_matches('/'))
                .to_lowercase();
            if candidate.as_ref().is_none_or(|item| item.0 < 4) && normalized_text.contains(&entry)
            {
                candidate = Some((
                    4,
                    Some(installation_id.clone()),
                    "entry-path",
                    "high",
                    snippet(&text),
                    Some(entry),
                ));
                break;
            }
            let matched_file = skill.files.iter().find(|file| {
                let full = format!("{}/{}", path.replace('\\', "/").trim_end_matches('/'), file)
                    .to_lowercase();
                normalized_text.contains(&full)
            });
            if candidate.as_ref().is_none_or(|item| item.0 < 3) {
                let Some(file) = matched_file else { continue };
                candidate = Some((
                    3,
                    Some(installation_id.clone()),
                    "bundle-path",
                    "high",
                    snippet(&text),
                    Some(file.clone()),
                ));
                break;
            }
        }
        if candidate.is_none() {
            if let Some(name) = command_names(&text)
                .into_iter()
                .find(|name| skill.names.contains(name))
            {
                candidate = Some((2, None, "explicit-name", "medium", name, None));
            } else if let Some(name) = skill
                .names
                .iter()
                .find(|name| normalized_text.contains(&format!("skill: {name}")))
            {
                candidate = Some((1, None, "content-evidence", "low", name.clone(), None));
            }
        }
        if let Some((priority, matched_installation, kind, confidence, evidence, path)) = candidate
        {
            let installation = matched_installation
                .or_else(|| skill.installations.first().map(|item| item.0.clone()));
            let value = invocation(event, skill, installation, kind, confidence, evidence, path);
            if matches
                .get(&skill.id)
                .is_none_or(|(current, _)| priority > *current)
            {
                matches.insert(skill.id.clone(), (priority, value));
            }
        }
    }
    matches.into_values().map(|(_, value)| value).collect()
}

fn invocation(
    event: &Event,
    skill: &SkillIdentity,
    installation_id: Option<String>,
    kind: &'static str,
    confidence: &'static str,
    evidence_text: String,
    evidence_path: Option<String>,
) -> Invocation {
    Invocation {
        skill_id: skill.id.clone(),
        installation_id,
        event_id: event.id.clone(),
        invoked_at_ms: event.timestamp.timestamp_millis(),
        detection_kind: kind,
        confidence,
        evidence_text: snippet(&evidence_text),
        evidence_path,
        token_count: event.metadata.usage.as_ref().map(|usage| {
            usage.input_tokens.unwrap_or(0)
                + usage.output_tokens.unwrap_or(0)
                + usage.cache_read_tokens.unwrap_or(0)
                + usage.cache_write_tokens.unwrap_or(0)
                + usage.reasoning_tokens.unwrap_or(0)
        }),
    }
}

fn explicit_names(event: &Event) -> BTreeSet<String> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::ToolCall {
                name,
                input: Some(input),
                ..
            } if matches!(
                normalize(name).as_str(),
                "skill" | "use-skill" | "use-skill-tool"
            ) =>
            {
                ["skill", "name", "command"]
                    .into_iter()
                    .find_map(|key| input.get(key).and_then(Value::as_str))
                    .map(normalize)
            }
            _ => None,
        })
        .collect()
}

fn command_names(text: &str) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for marker in ["<command-name>", "<skill-name>"] {
        let end = if marker == "<command-name>" {
            "</command-name>"
        } else {
            "</skill-name>"
        };
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            rest = &rest[start + marker.len()..];
            let Some(stop) = rest.find(end) else { break };
            result.insert(normalize(&rest[..stop]));
            rest = &rest[stop + end.len()..];
        }
    }
    result
}

fn event_text(event: &Event) -> String {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text { text } | Block::Thinking { text, .. } => Some(text.clone()),
            Block::ToolCall { name, input, .. } => Some(format!(
                "{} {}",
                name,
                input.as_ref().map(Value::to_string).unwrap_or_default()
            )),
            Block::ToolResult { content, .. } => Some(content.clone()),
            Block::Command { command, argv, .. } => Some(format!("{} {}", command, argv.join(" "))),
            Block::CommandResult {
                command,
                stdout,
                stderr,
                ..
            } => Some(format!(
                "{} {} {}",
                command.as_deref().unwrap_or(""),
                stdout.as_deref().unwrap_or(""),
                stderr.as_deref().unwrap_or("")
            )),
            Block::File { path, .. } => Some(path.clone()),
            Block::Patch { files, .. } => Some(files.join(" ")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_identities(conn: &Connection) -> Result<Vec<SkillIdentity>> {
    let mut statement = conn.prepare("SELECT c.id, c.bundle_content_hash, c.normalized_name, c.canonical_name, i.id, i.install_path, c.file_manifest_json FROM skill_catalog c JOIN skill_installations i ON i.skill_id = c.id WHERE i.status = 'active' ORDER BY c.id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut result = BTreeMap::<String, SkillIdentity>::new();
    for row in rows {
        let (id, fingerprint, normalized_name, canonical_name, installation_id, path, manifest) =
            row?;
        let item = result.entry(id.clone()).or_insert_with(|| SkillIdentity {
            id,
            fingerprint,
            names: BTreeSet::new(),
            installations: Vec::new(),
            files: Vec::new(),
        });
        item.names.insert(normalize(&normalized_name));
        item.names.insert(normalize(&canonical_name));
        item.installations.push((installation_id, path));
        for file in serde_json::from_str::<Vec<Value>>(&manifest).unwrap_or_default() {
            if let Some(path) = file
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.eq_ignore_ascii_case("SKILL.md"))
            {
                item.files.push(path.to_string());
            }
        }
    }
    Ok(result.into_values().collect())
}

pub fn catalog_generation(conn: &Connection) -> Result<String> {
    Ok(catalog_generation_from_identities(&load_identities(conn)?))
}

fn catalog_generation_from_identities(identities: &[SkillIdentity]) -> String {
    let mut hasher = Sha256::new();
    for identity in identities {
        hasher.update(identity.id.as_bytes());
        hasher.update([0]);
        hasher.update(identity.fingerprint.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn rebuild_daily(conn: &Connection, now_ms: i64) -> Result<()> {
    conn.execute("DELETE FROM skill_usage_daily", [])?;
    conn.execute(
        "INSERT INTO skill_usage_daily (usage_date, skill_id, provider_id, workspace_key, invocation_count, session_count, token_count, high_confidence_count, medium_confidence_count, low_confidence_count, updated_at_ms)
         SELECT date(invoked_at_ms / 1000, 'unixepoch', 'localtime'), skill_id, provider_id, COALESCE(workspace_dir, ''), COUNT(*), COUNT(DISTINCT session_id), CASE WHEN COUNT(token_count) = 0 THEN NULL ELSE SUM(token_count) END,
          SUM(confidence = 'high'), SUM(confidence = 'medium'), SUM(confidence = 'low'), ?1
         FROM skill_invocations GROUP BY 1, 2, 3, 4",
        [now_ms],
    )?;
    Ok(())
}

pub fn summary(conn: &Connection, query: &StatsQuery) -> Result<StatsSummary> {
    let values: (i64, i64, i64, i64, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT skill_id), COUNT(DISTINCT session_id),
                COUNT(DISTINCT date(invoked_at_ms / 1000, 'unixepoch', 'localtime')),
                CASE WHEN COUNT(token_count) = 0 THEN NULL ELSE SUM(token_count) END,
                MAX(invoked_at_ms)
         FROM skill_invocations
         WHERE (?1 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?1)
           AND (?2 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?2)
           AND (?3 IS NULL OR provider_id = ?3)
           AND (?4 IS NULL OR workspace_dir = ?4)
           AND (?5 IS NULL OR confidence = ?5)",
        params![
            query.from,
            query.to,
            query.provider,
            query.workspace,
            query.confidence
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    let completeness_status = conn
        .query_row(
            "SELECT completeness_status FROM skill_scan_state
             WHERE state_key = 'skill-sessions:all'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "unknown".into());
    Ok(StatsSummary {
        invocations: values.0 as u64,
        active_skills: values.1 as u64,
        active_sessions: values.2 as u64,
        active_days: values.3 as u64,
        token_count: values.4.map(|value| value as u64),
        last_invoked_at_ms: values.5,
        completeness_status,
    })
}

pub fn daily(
    conn: &Connection,
    query: &StatsQuery,
    skill_id: Option<&str>,
) -> Result<Vec<DailyUsage>> {
    let mut statement = conn.prepare(
        "SELECT date(invoked_at_ms / 1000, 'unixepoch', 'localtime'), COUNT(*),
                COUNT(DISTINCT session_id),
                CASE WHEN COUNT(token_count) = 0 THEN NULL ELSE SUM(token_count) END
         FROM skill_invocations
         WHERE (?1 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?1)
           AND (?2 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?2)
           AND (?3 IS NULL OR provider_id = ?3) AND (?4 IS NULL OR workspace_dir = ?4)
           AND (?5 IS NULL OR skill_id = ?5) AND (?6 IS NULL OR confidence = ?6)
         GROUP BY 1 ORDER BY 1",
    )?;
    let rows = statement.query_map(
        params![
            query.from,
            query.to,
            query.provider,
            query.workspace,
            skill_id,
            query.confidence
        ],
        |row| {
            Ok(DailyUsage {
                date: row.get(0)?,
                invocations: row.get::<_, i64>(1)? as u64,
                sessions: row.get::<_, i64>(2)? as u64,
                token_count: row.get::<_, Option<i64>>(3)?.map(|value| value as u64),
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn ranking(conn: &Connection, query: &StatsQuery) -> Result<Vec<SkillRanking>> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let mut statement = conn.prepare(
        "SELECT i.skill_id, c.canonical_name, COUNT(*), COUNT(DISTINCT i.session_id),
                CASE WHEN COUNT(i.token_count) = 0 THEN NULL ELSE SUM(i.token_count) END,
                MAX(i.invoked_at_ms)
         FROM skill_invocations i JOIN skill_catalog c ON c.id = i.skill_id
         WHERE (?1 IS NULL OR date(i.invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?1)
           AND (?2 IS NULL OR date(i.invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?2)
           AND (?3 IS NULL OR i.provider_id = ?3) AND (?4 IS NULL OR i.workspace_dir = ?4)
           AND (?5 IS NULL OR i.confidence = ?5)
         GROUP BY i.skill_id, c.canonical_name
         ORDER BY COUNT(*) DESC, c.canonical_name LIMIT ?6 OFFSET ?7",
    )?;
    let rows = statement.query_map(
        params![
            query.from,
            query.to,
            query.provider,
            query.workspace,
            query.confidence,
            page_size as i64,
            ((page - 1) * page_size) as i64
        ],
        |row| {
            Ok(SkillRanking {
                skill_id: row.get(0)?,
                name: row.get(1)?,
                invocations: row.get::<_, i64>(2)? as u64,
                sessions: row.get::<_, i64>(3)? as u64,
                token_count: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                last_invoked_at_ms: row.get(5)?,
                trend: Vec::new(),
            })
        },
    )?;
    let mut ranking = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for item in &mut ranking {
        item.trend = daily(conn, query, Some(&item.skill_id))?
            .into_iter()
            .map(|day| day.invocations)
            .collect();
    }
    Ok(ranking)
}

pub fn breakdown(conn: &Connection, query: &StatsQuery) -> Result<StatsBreakdown> {
    fn rows(
        conn: &Connection,
        query: &StatsQuery,
        column: &str,
    ) -> Result<Vec<StatsBreakdownItem>> {
        let sql = format!(
            "SELECT COALESCE(NULLIF({column}, ''), '未指定'), COUNT(*), COUNT(DISTINCT session_id)
             FROM skill_invocations
             WHERE (?1 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?1)
               AND (?2 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?2)
               AND (?3 IS NULL OR provider_id = ?3) AND (?4 IS NULL OR workspace_dir = ?4)
               AND (?5 IS NULL OR confidence = ?5)
             GROUP BY 1 ORDER BY COUNT(*) DESC, 1"
        );
        let mut statement = conn.prepare(&sql)?;
        let values = statement.query_map(
            params![
                query.from,
                query.to,
                query.provider,
                query.workspace,
                query.confidence
            ],
            |row| {
                Ok(StatsBreakdownItem {
                    key: row.get(0)?,
                    invocations: row.get::<_, i64>(1)? as u64,
                    sessions: row.get::<_, i64>(2)? as u64,
                })
            },
        )?;
        Ok(values.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    Ok(StatsBreakdown {
        providers: rows(conn, query, "provider_id")?,
        workspaces: rows(conn, query, "workspace_dir")?,
    })
}

pub fn invocations(
    conn: &Connection,
    skill_id: &str,
    query: &StatsQuery,
) -> Result<InvocationPage> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM skill_invocations WHERE skill_id = ?1
         AND (?2 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?2)
         AND (?3 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?3)
         AND (?4 IS NULL OR provider_id = ?4) AND (?5 IS NULL OR workspace_dir = ?5)
         AND (?6 IS NULL OR confidence = ?6)",
        params![
            skill_id,
            query.from,
            query.to,
            query.provider,
            query.workspace,
            query.confidence
        ],
        |row| row.get(0),
    )?;
    let mut statement = conn.prepare(
        "SELECT id, session_id, event_id, provider_id, workspace_dir, invoked_at_ms,
                detection_kind, confidence, evidence_text, evidence_path, token_count
         FROM skill_invocations WHERE skill_id = ?1
         AND (?2 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') >= ?2)
         AND (?3 IS NULL OR date(invoked_at_ms / 1000, 'unixepoch', 'localtime') <= ?3)
         AND (?4 IS NULL OR provider_id = ?4) AND (?5 IS NULL OR workspace_dir = ?5)
         AND (?6 IS NULL OR confidence = ?6)
         ORDER BY invoked_at_ms DESC LIMIT ?7 OFFSET ?8",
    )?;
    let rows = statement.query_map(
        params![
            skill_id,
            query.from,
            query.to,
            query.provider,
            query.workspace,
            query.confidence,
            page_size as i64,
            ((page - 1) * page_size) as i64
        ],
        |row| {
            Ok(InvocationItem {
                id: row.get(0)?,
                session_id: row.get(1)?,
                event_id: row.get(2)?,
                provider_id: row.get(3)?,
                workspace_dir: row.get(4)?,
                invoked_at_ms: row.get(5)?,
                detection_kind: row.get(6)?,
                confidence: row.get(7)?,
                evidence_text: row.get(8)?,
                evidence_path: row.get(9)?,
                token_count: row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
            })
        },
    )?;
    Ok(InvocationPage {
        items: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        page,
        page_size,
        total: total as usize,
    })
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('/')
        .to_lowercase()
        .replace(['_', ' '], "-")
}
fn snippet(value: &str) -> String {
    value.chars().take(240).collect()
}
fn hash(bytes: &[u8]) -> String {
    format!("invocation:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{EventKind, Links, Metadata, Role};
    use chrono::TimeZone;

    fn event(blocks: Vec<Block>) -> Event {
        Event {
            id: "event-1".into(),
            kind: EventKind::Action,
            role: Role::Assistant,
            timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            links: Links::default(),
            blocks,
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        }
    }

    #[test]
    fn breakdown_applies_filters_and_counts_distinct_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE skill_invocations (
                invoked_at_ms INTEGER NOT NULL,
                session_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                workspace_dir TEXT,
                confidence TEXT NOT NULL
             );
             INSERT INTO skill_invocations VALUES
                (1784656800000, 's1', 'codex', '/work/a', 'high'),
                (1784656801000, 's1', 'codex', '/work/a', 'high'),
                (1784656802000, 's2', 'claude', '/work/b', 'low');",
        )
        .unwrap();
        let result = breakdown(
            &conn,
            &StatsQuery {
                provider: Some("codex".into()),
                confidence: Some("high".into()),
                ..StatsQuery::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.providers,
            vec![StatsBreakdownItem {
                key: "codex".into(),
                invocations: 2,
                sessions: 1,
            }]
        );
        assert_eq!(result.workspaces[0].key, "/work/a");
        assert_eq!(result.workspaces[0].invocations, 2);
    }

    #[test]
    fn keeps_only_highest_priority_evidence_per_event_and_skill() {
        let skill = SkillIdentity {
            id: "skill-1".into(),
            fingerprint: "bundle-1".into(),
            names: BTreeSet::from(["demo".into()]),
            installations: vec![("install-1".into(), "/tmp/demo".into())],
            files: vec!["scripts/run.sh".into()],
        };
        let found = detect_event(
            &event(vec![Block::ToolCall {
                tool_call_id: "1".into(),
                name: "skill".into(),
                input: Some(serde_json::json!({"skill":"demo", "path":"/tmp/demo/SKILL.md"})),
            }]),
            &[skill],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].detection_kind, "explicit-tool");
        assert_eq!(found[0].confidence, "high");
    }

    #[test]
    fn cursor_only_source_change_is_reindexed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            crate::storage::local_store::LocalSqliteStore::open(dir.path().join("memorph.db"))
                .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO session_sources
                 (id, provider_id, source_path, file_mtime_ms, file_size_bytes,
                  source_cursor, first_seen_at_ms, last_seen_at_ms)
                 VALUES ('source', 'cursor', '/tmp/source', 1, 2, 'cursor-1', 1, 1)",
                [],
            )
            .unwrap();

        let first = index(store.connection_mut(), false).unwrap();
        let unchanged = index(store.connection_mut(), false).unwrap();
        store
            .connection()
            .execute(
                "UPDATE session_sources SET source_cursor = 'cursor-2' WHERE id = 'source'",
                [],
            )
            .unwrap();
        let cursor_changed = index(store.connection_mut(), false).unwrap();

        assert_eq!(first.sources_scanned, 1);
        assert_eq!(unchanged.sources_skipped, 1);
        assert_eq!(cursor_changed.sources_scanned, 1);
        assert_eq!(cursor_changed.sources_skipped, 0);
    }

    #[test]
    fn catalog_generation_change_marks_complete_source_stale() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            crate::storage::local_store::LocalSqliteStore::open(dir.path().join("memorph.db"))
                .unwrap();
        let conn = store.connection_mut();
        conn.execute(
            "INSERT INTO skill_catalog
             (id, canonical_name, normalized_name, entry_content_hash, bundle_content_hash,
              file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms, created_at_ms, updated_at_ms)
             VALUES ('skill-1', 'Skill 1', 'skill-1', 'entry-1', 'bundle-1', 1, 1, 1, 1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO skill_installations
             (id, skill_id, used_by, scope_kind, install_path, canonical_install_path,
              install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
             VALUES ('install-1', 'skill-1', 'codex', 'global', '/tmp/skill-1', '/tmp/skill-1',
                     'directory', 'bundle-1', 1, 1)",
            [],
        )
        .unwrap();
        let generation = catalog_generation(conn).unwrap();
        conn.execute(
            "INSERT INTO skill_scan_state
             (state_key, state_kind, source_fingerprint, source_cursor, completeness_status,
              details_json, updated_at_ms)
             VALUES ('session-source:source', 'session-source', 'fingerprint', 'cursor',
                     'complete', ?1, 1)",
            [serde_json::json!({"catalog_generation": generation}).to_string()],
        )
        .unwrap();
        assert!(repository::session_source_scan_is_current(
            conn,
            "session-source:source",
            "fingerprint",
            Some("cursor"),
            &generation
        )
        .unwrap());

        conn.execute(
            "INSERT INTO skill_catalog
             (id, canonical_name, normalized_name, entry_content_hash, bundle_content_hash,
              file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms, created_at_ms, updated_at_ms)
             VALUES ('skill-2', 'Skill 2', 'skill-2', 'entry-2', 'bundle-2', 1, 1, 1, 1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO skill_installations
             (id, skill_id, used_by, scope_kind, install_path, canonical_install_path,
              install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
             VALUES ('install-2', 'skill-2', 'codex', 'global', '/tmp/skill-2', '/tmp/skill-2',
                     'directory', 'bundle-2', 1, 1)",
            [],
        )
        .unwrap();
        let changed = catalog_generation(conn).unwrap();
        assert_ne!(generation, changed);
        assert!(!repository::session_source_scan_is_current(
            conn,
            "session-source:source",
            "fingerprint",
            Some("cursor"),
            &changed
        )
        .unwrap());
    }
}
