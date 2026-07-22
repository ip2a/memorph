use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::{fs, path::Path};
use walkdir::WalkDir;

pub const ALGORITHM_VERSION: &str = "unicode-mixed-v1";

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ContextLayer {
    pub bytes: u64,
    pub characters: u64,
    pub token_lower: u64,
    pub estimated_tokens: u64,
    pub token_upper: u64,
    pub estimated: bool,
    pub algorithm_version: &'static str,
    pub baseline_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SkillContext {
    pub skill_id: String,
    pub name: String,
    pub metadata: ContextLayer,
    pub body: ContextLayer,
    pub auxiliary: ContextLayer,
    pub observed_token_min: Option<u64>,
    pub observed_token_max: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ContextSummary {
    pub baseline_tokens: Option<u64>,
    pub algorithm_version: &'static str,
    pub skills: Vec<SkillContext>,
}

pub fn summary(
    conn: &Connection,
    provider: Option<&str>,
    baseline: Option<u64>,
) -> Result<ContextSummary> {
    let mut statement = conn.prepare(
        "SELECT c.id, c.canonical_name, c.metadata_json, i.install_path,
                (SELECT MIN(token_count) FROM skill_invocations v WHERE v.skill_id = c.id AND token_count IS NOT NULL),
                (SELECT MAX(token_count) FROM skill_invocations v WHERE v.skill_id = c.id AND token_count IS NOT NULL)
         FROM skill_catalog c JOIN skill_installations i ON i.id = (
           SELECT id FROM skill_installations WHERE skill_id = c.id AND status = 'active'
             AND (?1 IS NULL OR provider_id = ?1) ORDER BY provider_id, id LIMIT 1)
         WHERE c.missing_since_ms IS NULL ORDER BY c.canonical_name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([provider], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut skills = Vec::new();
    for row in rows {
        let (id, name, metadata_json, install_path, observed_min, observed_max) = row?;
        skills.push(build(
            id,
            name,
            &metadata_json,
            Path::new(&install_path),
            baseline,
            observed_min.map(|v| v as u64),
            observed_max.map(|v| v as u64),
        ));
    }
    Ok(ContextSummary {
        baseline_tokens: baseline,
        algorithm_version: ALGORITHM_VERSION,
        skills,
    })
}

pub fn detail(conn: &Connection, skill_id: &str, baseline: Option<u64>) -> Result<SkillContext> {
    let (name, metadata_json, install_path, observed_min, observed_max) = conn
        .query_row(
            "SELECT c.canonical_name, c.metadata_json, i.install_path,
                    (SELECT MIN(token_count) FROM skill_invocations v WHERE v.skill_id = c.id AND token_count IS NOT NULL),
                    (SELECT MAX(token_count) FROM skill_invocations v WHERE v.skill_id = c.id AND token_count IS NOT NULL)
             FROM skill_catalog c JOIN skill_installations i ON i.id = (
               SELECT id FROM skill_installations WHERE skill_id = c.id AND status = 'active' ORDER BY provider_id, id LIMIT 1)
             WHERE c.id = ?1",
            [skill_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?)),
        )
        .context("Skill not found")?;
    Ok(build(
        skill_id.to_string(),
        name,
        &metadata_json,
        Path::new(&install_path),
        baseline,
        observed_min.map(|v| v as u64),
        observed_max.map(|v| v as u64),
    ))
}

fn build(
    skill_id: String,
    name: String,
    metadata_json: &str,
    root: &Path,
    baseline: Option<u64>,
    observed_token_min: Option<u64>,
    observed_token_max: Option<u64>,
) -> SkillContext {
    let metadata = serde_json::from_str::<Value>(metadata_json)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|map| {
            map.into_iter()
                .map(|(key, value)| format!("{key}: {}", value.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let entry = fs::read_to_string(root.join("SKILL.md")).unwrap_or_default();
    let body = entry
        .strip_prefix("---")
        .and_then(|rest| rest.split_once("---").map(|(_, body)| body))
        .unwrap_or(&entry);
    let auxiliary = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() != "SKILL.md")
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .collect::<Vec<_>>()
        .join("\n");
    SkillContext {
        skill_id,
        name,
        metadata: estimate(&metadata, baseline),
        body: estimate(body, baseline),
        auxiliary: estimate(&auxiliary, baseline),
        observed_token_min,
        observed_token_max,
    }
}

pub(crate) fn estimate(text: &str, baseline: Option<u64>) -> ContextLayer {
    let mut cjk = 0_f64;
    let mut code = 0_f64;
    let mut latin = 0_f64;
    for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
        if matches!(ch as u32, 0x3400..=0x9fff | 0x3040..=0x30ff | 0xac00..=0xd7af) {
            cjk += 1.0;
        } else if ch.is_ascii_punctuation() || ch.is_ascii_digit() {
            code += 1.0;
        } else {
            latin += 1.0;
        }
    }
    let value = cjk / 1.7 + code / 3.0 + latin / 4.0;
    let estimated_tokens = value.ceil() as u64;
    ContextLayer {
        bytes: text.len() as u64,
        characters: text.chars().count() as u64,
        token_lower: (cjk / 2.0 + code / 4.0 + latin / 5.0).ceil() as u64,
        estimated_tokens,
        token_upper: (cjk + code / 2.0 + latin / 3.0).ceil() as u64,
        estimated: true,
        algorithm_version: ALGORITHM_VERSION,
        baseline_percent: baseline
            .filter(|value| *value > 0)
            .map(|value| estimated_tokens as f64 * 100.0 / value as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::estimate;

    #[test]
    fn estimates_cjk_latin_and_code_differently() {
        let en = estimate("abcdefghij", None);
        let cjk = estimate("编写简洁技术文档说明", None);
        let code = estimate("fn main() { println!(\"ok\"); }", None);
        assert!(cjk.estimated_tokens > en.estimated_tokens);
        assert!(code.estimated_tokens > 0);
        assert!(en.token_lower <= en.estimated_tokens && en.estimated_tokens <= en.token_upper);
    }
}
