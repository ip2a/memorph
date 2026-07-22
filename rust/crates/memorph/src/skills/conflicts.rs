use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};

const NAME_THRESHOLD: f64 = 0.8;
const DESCRIPTION_THRESHOLD: f64 = 0.65;
const STOP_WORDS: &[&str] = &["a", "an", "and", "for", "of", "the", "to", "with"];

#[derive(Clone, Debug)]
struct Skill {
    id: String,
    name: String,
    description: String,
    bundle_hash: String,
    triggers: Vec<String>,
    providers: BTreeSet<String>,
    scopes: BTreeSet<String>,
    install_hashes: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Conflict {
    pub id: String,
    pub left_skill_id: String,
    pub left_name: String,
    pub right_skill_id: String,
    pub right_name: String,
    pub conflict_kind: String,
    pub severity: String,
    pub similarity: f64,
    pub overlapping_tokens: Vec<String>,
    pub evidence: String,
    pub recommendation: String,
}

pub fn list(conn: &Connection, skill_id: Option<&str>) -> Result<Vec<Conflict>> {
    let skills = skills(conn)?;
    let mut result = Vec::new();
    for (index, left) in skills.iter().enumerate() {
        for right in &skills[index + 1..] {
            if skill_id.is_some_and(|id| id != left.id && id != right.id) {
                continue;
            }
            if let Some(conflict) = compare(left, right) {
                result.push(conflict);
            }
        }
        if left.providers.len() > 1 && left.install_hashes.len() == 1 {
            result.push(Conflict {
                id: format!("{}:provider-duplicate", left.id),
                left_skill_id: left.id.clone(),
                left_name: left.name.clone(),
                right_skill_id: left.id.clone(),
                right_name: left.name.clone(),
                conflict_kind: "provider-duplicate".into(),
                severity: "info".into(),
                similarity: 1.0,
                overlapping_tokens: left.providers.iter().cloned().collect(),
                evidence: "多个 Provider 安装相同 Bundle 内容".into(),
                recommendation: "无需处理；仅在不再需要时移除重复链接".into(),
            });
        }
    }
    Ok(result)
}

fn skills(conn: &Connection) -> Result<Vec<Skill>> {
    let mut statement = conn.prepare(
        "SELECT c.id, c.canonical_name, COALESCE(c.description, ''), c.bundle_content_hash,
                c.trigger_terms_json, i.provider_id, i.scope_kind, i.bundle_content_hash
         FROM skill_catalog c JOIN skill_installations i ON i.skill_id = c.id AND i.status = 'active'
         WHERE c.missing_since_ms IS NULL ORDER BY c.id, i.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut result = Vec::<Skill>::new();
    for row in rows {
        let (id, name, description, bundle_hash, triggers, provider, scope, install_hash) = row?;
        let current = if result.last().is_some_and(|item| item.id == id) {
            result.last_mut().unwrap()
        } else {
            result.push(Skill {
                id,
                name,
                description,
                bundle_hash,
                triggers: serde_json::from_str(&triggers).unwrap_or_default(),
                providers: BTreeSet::new(),
                scopes: BTreeSet::new(),
                install_hashes: BTreeSet::new(),
            });
            result.last_mut().unwrap()
        };
        current.providers.insert(provider);
        current.scopes.insert(scope);
        current.install_hashes.insert(install_hash);
    }
    Ok(result)
}

fn compare(left: &Skill, right: &Skill) -> Option<Conflict> {
    let left_triggers = left
        .triggers
        .iter()
        .map(|value| normalize(value))
        .collect::<HashSet<_>>();
    let right_triggers = right
        .triggers
        .iter()
        .map(|value| normalize(value))
        .collect::<HashSet<_>>();
    let shared_triggers = left_triggers
        .intersection(&right_triggers)
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let left_name = normalize(&left.name);
    let right_name = normalize(&right.name);
    let name_tokens = tokens(&left.name);
    let right_name_tokens = tokens(&right.name);
    let description_tokens = tokens(&left.description);
    let right_description_tokens = tokens(&right.description);
    let (kind, severity, similarity, overlap, evidence) = if !shared_triggers.is_empty() {
        (
            "exact-trigger",
            "error",
            1.0,
            shared_triggers.clone(),
            format!("相同触发词: {}", shared_triggers.join(", ")),
        )
    } else if left_name == right_name
        && left.bundle_hash != right.bundle_hash
        && left
            .scopes
            .union(&right.scopes)
            .any(|scope| scope == "global")
        && left
            .scopes
            .union(&right.scopes)
            .any(|scope| scope == "project")
    {
        (
            "cross-scope-shadow",
            "warning",
            1.0,
            vec![left_name],
            "全局与项目范围同名但内容不同".into(),
        )
    } else if left_name == right_name {
        (
            "normalized-name",
            "warning",
            1.0,
            vec![left_name],
            "规范化名称相同".into(),
        )
    } else {
        let name_score = jaccard(&name_tokens, &right_name_tokens);
        let description_score = jaccard(&description_tokens, &right_description_tokens);
        if name_score >= NAME_THRESHOLD {
            (
                "name-overlap",
                "warning",
                name_score,
                intersection(&name_tokens, &right_name_tokens),
                format!("名称 token 重叠 {:.0}%", name_score * 100.0),
            )
        } else if description_tokens.len() >= 3
            && right_description_tokens.len() >= 3
            && description_score >= DESCRIPTION_THRESHOLD
        {
            (
                "description-similarity",
                "warning",
                description_score,
                intersection(&description_tokens, &right_description_tokens),
                format!("描述 token 相似 {:.0}%", description_score * 100.0),
            )
        } else {
            return None;
        }
    };
    Some(Conflict {
        id: format!("{}:{}:{kind}", left.id, right.id),
        left_skill_id: left.id.clone(),
        left_name: left.name.clone(),
        right_skill_id: right.id.clone(),
        right_name: right.name.clone(),
        conflict_kind: kind.into(),
        severity: severity.into(),
        similarity,
        overlapping_tokens: overlap,
        evidence,
        recommendation: "细化名称、描述或显式触发条件".into(),
    })
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(['/', '@', ':'])
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokens(value: &str) -> BTreeSet<String> {
    let normalized = normalize(value);
    let mut result = normalized
        .split_whitespace()
        .filter(|word| !STOP_WORDS.contains(word))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    for run in normalized
        .split_whitespace()
        .filter(|word| word.chars().all(|ch| matches!(ch as u32, 0x3400..=0x9fff)))
    {
        let chars = run.chars().collect::<Vec<_>>();
        result.extend(chars.windows(2).map(|pair| pair.iter().collect()));
    }
    result
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        0.0
    } else {
        left.intersection(right).count() as f64 / union as f64
    }
}
fn intersection(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn skill(id: &str, name: &str, description: &str, triggers: &[&str]) -> Skill {
        Skill {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            bundle_hash: id.into(),
            triggers: triggers.iter().map(|v| v.to_string()).collect(),
            providers: BTreeSet::new(),
            scopes: BTreeSet::new(),
            install_hashes: BTreeSet::new(),
        }
    }
    #[test]
    fn exact_trigger_wins_and_cjk_bigrams_overlap() {
        let conflict = compare(
            &skill("a", "文档写作", "生成技术文档说明", &["/docs"]),
            &skill("b", "文档编写", "编写技术文档说明", &["docs"]),
        )
        .unwrap();
        assert_eq!(conflict.conflict_kind, "exact-trigger");
        assert!(tokens("生成技术文档").contains("文档"));
    }
}
