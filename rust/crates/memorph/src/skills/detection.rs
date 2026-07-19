use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    model::{SkillRelationEvidence, SkillRelationKind, SkillSelector},
    server::{SkillEntry, SkillsOverview},
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillRelationCandidate {
    pub key: String,
    pub from: SkillSelector,
    pub to: SkillSelector,
    pub kind: SkillRelationKind,
    pub confidence: u8,
    pub evidence: SkillRelationEvidence,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillGroupCandidate {
    pub key: String,
    pub suggested_id: String,
    pub name: String,
    pub members: Vec<String>,
    pub confidence: u8,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillDetectionResult {
    pub relations: Vec<SkillRelationCandidate>,
    pub groups: Vec<SkillGroupCandidate>,
}

pub fn detect(overview: &SkillsOverview) -> SkillDetectionResult {
    let known = overview
        .skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill))
        .collect::<BTreeMap<_, _>>();
    let mut relations = Vec::new();

    for skill in &overview.skills {
        let Some(source) = skill.installations.first() else {
            continue;
        };
        let path = source.path.join("SKILL.md");
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in contents.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let kind = classify_line(&lower);
            for target in known.keys().copied() {
                if target == skill.id || !mentions_skill(line, target) {
                    continue;
                }
                relations.push(candidate(
                    skill,
                    known[target],
                    kind.clone(),
                    if explicit_skill_marker(line, target) {
                        95
                    } else {
                        82
                    },
                    index + 1,
                    line,
                ));
            }
            for pattern in wildcard_patterns(line) {
                for target in known
                    .keys()
                    .copied()
                    .filter(|id| wildcard_match(&pattern, id))
                {
                    if target == skill.id {
                        continue;
                    }
                    relations.push(candidate(
                        skill,
                        known[target],
                        SkillRelationKind::Orchestrates,
                        90,
                        index + 1,
                        line,
                    ));
                }
            }
        }
    }

    relations.sort_by(|left, right| left.key.cmp(&right.key));
    relations.dedup_by(|left, right| left.key == right.key);
    SkillDetectionResult {
        relations,
        groups: detect_groups(&overview.skills),
    }
}

fn candidate(
    from: &SkillEntry,
    to: &SkillEntry,
    kind: SkillRelationKind,
    confidence: u8,
    line: usize,
    excerpt: &str,
) -> SkillRelationCandidate {
    let from_selector = selector(from);
    let to_selector = selector(to);
    let evidence = SkillRelationEvidence {
        path: "SKILL.md".into(),
        line: Some(line),
        excerpt: excerpt.trim().chars().take(240).collect(),
    };
    let mut hasher = Sha256::new();
    hasher.update(from.id.as_bytes());
    hasher.update(format!("{kind:?}").as_bytes());
    hasher.update(to.id.as_bytes());
    hasher.update(evidence.excerpt.as_bytes());
    SkillRelationCandidate {
        key: format!("sha256:{:x}", hasher.finalize()),
        from: from_selector,
        to: to_selector,
        kind,
        confidence,
        evidence,
    }
}

fn selector(skill: &SkillEntry) -> SkillSelector {
    SkillSelector {
        skill_id: skill.id.clone(),
        source_provider: None,
        fingerprint: if skill.conflict {
            Some(skill.fingerprint.clone())
        } else {
            None
        },
    }
}

fn classify_line(line: &str) -> SkillRelationKind {
    if line.contains("fallback") || line.contains("fall back") {
        SkillRelationKind::FallbackTo
    } else if line.contains("route") || line.contains("handoff") || line.contains("hand off") {
        SkillRelationKind::RoutesTo
    } else if line.contains("require") || line.contains("depends") {
        SkillRelationKind::Requires
    } else if line.contains("all ") || line.contains("orchestrat") || line.contains("delegate") {
        SkillRelationKind::Orchestrates
    } else {
        SkillRelationKind::Uses
    }
}

fn mentions_skill(line: &str, skill_id: &str) -> bool {
    explicit_skill_marker(line, skill_id)
        || line
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '-' || ch == '_'))
            .any(|token| token.eq_ignore_ascii_case(skill_id))
}

fn explicit_skill_marker(line: &str, skill_id: &str) -> bool {
    line.contains(&format!("${skill_id}"))
        || line.contains(&format!("`{skill_id}`"))
        || line.contains(&format!("skills/{skill_id}"))
        || line.contains(&format!("{skill_id}/SKILL.md"))
}

fn wildcard_patterns(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                !(ch.is_alphanumeric() || ch == '-' || ch == '*' || ch == '_')
            })
        })
        .filter(|token| token.contains('*') && token.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return false;
    };
    value.starts_with(prefix) && value.ends_with(suffix)
}

fn detect_groups(skills: &[SkillEntry]) -> Vec<SkillGroupCandidate> {
    let mut prefixes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for skill in skills {
        let Some((prefix, _)) = skill.id.split_once('-') else {
            continue;
        };
        prefixes
            .entry(prefix.to_string())
            .or_default()
            .insert(skill.id.clone());
    }
    prefixes
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(prefix, members)| SkillGroupCandidate {
            key: format!("prefix:{prefix}"),
            suggested_id: format!("{prefix}-suite"),
            name: format!("{} Suite", title_case(&prefix)),
            members: members.into_iter().collect(),
            confidence: 45,
            evidence: format!("Shared normalized name prefix: {prefix}-"),
        })
        .collect()
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching_is_deterministic() {
        assert!(wildcard_match("code-review-*", "code-review-testing"));
        assert!(!wildcard_match("code-review-*", "code-review"));
    }

    #[test]
    fn relation_phrases_map_to_specific_kinds() {
        assert_eq!(
            classify_line("route to `docx`"),
            SkillRelationKind::RoutesTo
        );
        assert_eq!(
            classify_line("requires $delegate"),
            SkillRelationKind::Requires
        );
        assert_eq!(
            classify_line("fallback to other"),
            SkillRelationKind::FallbackTo
        );
    }
}
