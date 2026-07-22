use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
struct Section {
    id: String,
    title: String,
}
#[derive(Clone, Debug, Deserialize)]
struct Asset {
    path: String,
    category: String,
    entry: bool,
}
#[derive(Clone, Debug)]
struct Target {
    kind: String,
    key: String,
    path: Option<String>,
    title: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CoverageTarget {
    pub target_kind: String,
    pub target_key: String,
    pub target_path: Option<String>,
    pub section_title: Option<String>,
    pub confidence: Option<String>,
    pub observations: u64,
    pub first_observed_at_ms: Option<i64>,
    pub last_observed_at_ms: Option<i64>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SkillCoverage {
    pub skill_id: String,
    pub covered: u64,
    pub total: u64,
    pub percent: f64,
    pub completeness_status: String,
    pub targets: Vec<CoverageTarget>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CoverageSummaryItem {
    pub skill_id: String,
    pub name: String,
    pub covered: u64,
    pub total: u64,
    pub percent: f64,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CoverageEvidence {
    pub invocation_id: String,
    pub session_id: String,
    pub provider_id: String,
    pub event_id: Option<String>,
    pub observed_at_ms: i64,
    pub match_kind: String,
    pub confidence: String,
    pub evidence_text: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CoverageEvidencePage {
    pub items: Vec<CoverageEvidence>,
    pub page: usize,
    pub page_size: usize,
    pub total: u64,
}

pub fn rebuild(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM skill_coverage_observations", [])?;
    let mut statement = conn.prepare("SELECT i.id, i.skill_id, i.invoked_at_ms, COALESCE(i.evidence_text, ''), COALESCE(i.evidence_path, ''), c.section_index_json, c.file_manifest_json FROM skill_invocations i JOIN skill_catalog c ON c.id = i.skill_id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    for row in rows {
        let (invocation_id, skill_id, at, text, evidence_path, sections, assets) = row?;
        let targets = targets(&sections, &assets);
        let haystack = normalize(&format!("{text} {evidence_path}"));
        let mut basename_counts = BTreeMap::new();
        for target in &targets {
            if let Some(path) = &target.path {
                *basename_counts
                    .entry(path.rsplit('/').next().unwrap_or(path).to_string())
                    .or_insert(0) += 1;
            }
        }
        for target in targets {
            let matched = target
                .path
                .as_deref()
                .and_then(|path| {
                    let normalized = normalize(path);
                    if haystack.contains(&normalized) {
                        Some((
                            if text.contains(path) || evidence_path.contains(path) {
                                "exact-path"
                            } else {
                                "normalized-path"
                            },
                            "high",
                        ))
                    } else {
                        let base = path.rsplit('/').next().unwrap_or(path);
                        (basename_counts.get(base) == Some(&1)
                            && haystack.contains(&normalize(base)))
                        .then_some(("unique-basename", "medium"))
                    }
                })
                .or_else(|| {
                    target
                        .title
                        .as_deref()
                        .filter(|title| haystack.contains(&normalize(title)))
                        .map(|_| ("section-anchor", "medium"))
                });
            if let Some((kind, confidence)) = matched {
                let id = format!(
                    "coverage:{:x}",
                    Sha256::digest(format!("{invocation_id}:{}:{kind}", target.key).as_bytes())
                );
                conn.execute("INSERT OR IGNORE INTO skill_coverage_observations (id, skill_id, invocation_id, target_kind, target_key, target_path, section_title, match_kind, confidence, observed_at_ms, evidence_text, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?10)", params![id, skill_id, invocation_id, target.kind, target.key, target.path, target.title, kind, confidence, at, text.chars().take(240).collect::<String>()])?;
            }
        }
    }
    Ok(())
}

pub fn detail(
    conn: &Connection,
    skill_id: &str,
    range: &str,
    include_low: bool,
) -> Result<SkillCoverage> {
    let (sections, assets): (String, String) = conn.query_row(
        "SELECT section_index_json, file_manifest_json FROM skill_catalog WHERE id = ?1",
        [skill_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let since = match range {
        "7d" => Some(chrono::Utc::now().timestamp_millis() - 7 * 86_400_000),
        "30d" => Some(chrono::Utc::now().timestamp_millis() - 30 * 86_400_000),
        "90d" => Some(chrono::Utc::now().timestamp_millis() - 90 * 86_400_000),
        _ => None,
    };
    let mut result = Vec::new();
    for target in targets(&sections, &assets) {
        let values = conn.query_row("SELECT COUNT(*), MIN(observed_at_ms), MAX(observed_at_ms), MAX(CASE confidence WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END) FROM skill_coverage_observations WHERE skill_id = ?1 AND target_kind = ?2 AND target_key = ?3 AND (?4 OR confidence <> 'low') AND (?5 IS NULL OR observed_at_ms >= ?5)", params![skill_id, target.kind, target.key, include_low, since], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, Option<i64>>(2)?, row.get::<_, Option<i64>>(3)?)))?;
        result.push(CoverageTarget {
            target_kind: target.kind,
            target_key: target.key,
            target_path: target.path,
            section_title: target.title,
            confidence: values.3.map(|v| {
                if v == 3 {
                    "high"
                } else if v == 2 {
                    "medium"
                } else {
                    "low"
                }
                .into()
            }),
            observations: values.0 as u64,
            first_observed_at_ms: values.1,
            last_observed_at_ms: values.2,
        });
    }
    let total = result.len() as u64;
    let covered = result
        .iter()
        .filter(|item| item.observations > 0 && item.confidence.as_deref() != Some("low"))
        .count() as u64;
    let completeness_status = conn.query_row("SELECT completeness_status FROM skill_scan_state WHERE state_key = 'skill-sessions:all'", [], |row| row.get(0)).unwrap_or_else(|_| "unknown".into());
    Ok(SkillCoverage {
        skill_id: skill_id.into(),
        covered,
        total,
        percent: if total == 0 {
            0.0
        } else {
            covered as f64 * 100.0 / total as f64
        },
        completeness_status,
        targets: result,
    })
}

pub fn summary(conn: &Connection, range: &str) -> Result<Vec<CoverageSummaryItem>> {
    let mut statement = conn.prepare("SELECT id, canonical_name FROM skill_catalog WHERE missing_since_ms IS NULL ORDER BY canonical_name")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (id, name) = row?;
        let value = detail(conn, &id, range, false)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        Ok(CoverageSummaryItem {
            skill_id: id,
            name,
            covered: value.covered,
            total: value.total,
            percent: value.percent,
        })
    })
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(Into::into)
}

pub fn evidence(
    conn: &Connection,
    skill_id: &str,
    target_key: &str,
    page: usize,
    page_size: usize,
) -> Result<CoverageEvidencePage> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let total = conn.query_row(
        "SELECT COUNT(*) FROM skill_coverage_observations WHERE skill_id = ?1 AND target_key = ?2",
        params![skill_id, target_key],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut statement = conn.prepare("SELECT o.invocation_id, i.session_id, i.provider_id, i.event_id, o.observed_at_ms, o.match_kind, o.confidence, o.evidence_text FROM skill_coverage_observations o JOIN skill_invocations i ON i.id = o.invocation_id WHERE o.skill_id = ?1 AND o.target_key = ?2 ORDER BY o.observed_at_ms DESC LIMIT ?3 OFFSET ?4")?;
    let rows = statement.query_map(
        params![
            skill_id,
            target_key,
            page_size as i64,
            ((page - 1) * page_size) as i64
        ],
        |row| {
            Ok(CoverageEvidence {
                invocation_id: row.get(0)?,
                session_id: row.get(1)?,
                provider_id: row.get(2)?,
                event_id: row.get(3)?,
                observed_at_ms: row.get(4)?,
                match_kind: row.get(5)?,
                confidence: row.get(6)?,
                evidence_text: row.get(7)?,
            })
        },
    )?;
    Ok(CoverageEvidencePage {
        items: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        page,
        page_size,
        total,
    })
}

fn targets(sections: &str, assets: &str) -> Vec<Target> {
    let mut result = serde_json::from_str::<Vec<Section>>(sections)
        .unwrap_or_default()
        .into_iter()
        .map(|item| Target {
            kind: "section".into(),
            key: item.id,
            path: None,
            title: Some(item.title),
        })
        .collect::<Vec<_>>();
    result.extend(
        serde_json::from_str::<Vec<Asset>>(assets)
            .unwrap_or_default()
            .into_iter()
            .filter(|item| !item.entry)
            .map(|item| Target {
                kind: match item.category.as_str() {
                    "script" => "script",
                    "reference" => "reference",
                    "asset" => "asset",
                    _ => "other-file",
                }
                .into(),
                key: item.path.clone(),
                path: Some(item.path),
                title: None,
            }),
    );
    result
}
fn normalize(value: &str) -> String {
    value.replace("%20", " ").replace('\\', "/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_section_identity_and_rejects_ambiguous_basenames() {
        let sections = r#"[{"id":"stable","title":"Setup"}]"#;
        let assets = r#"[{"path":"a/run.sh","category":"script","entry":false},{"path":"b/run.sh","category":"script","entry":false}]"#;
        let values = targets(sections, assets);
        assert_eq!(values[0].key, "stable");
        let counts = values
            .iter()
            .filter_map(|item| item.path.as_deref())
            .filter(|path| path.ends_with("run.sh"))
            .count();
        assert_eq!(counts, 2);
    }
}
