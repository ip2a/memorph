use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use crate::{config, storage::atomic_write};

use super::model::{
    IgnoredSkillCandidate, SkillGroup, SkillRelationRule, SkillRelationsConfig,
    SKILL_RELATIONS_SCHEMA_VERSION,
};

pub fn relations_path() -> Result<PathBuf> {
    Ok(config::memorph_dir()?.join("skills.json"))
}

pub fn load() -> Result<SkillRelationsConfig> {
    let path = relations_path()?;
    if !path.exists() {
        return Ok(SkillRelationsConfig::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read skill relations: {}", path.display()))?;
    let config: SkillRelationsConfig = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse skill relations: {}", path.display()))?;
    if config.schema_version != SKILL_RELATIONS_SCHEMA_VERSION {
        bail!(
            "Unsupported skill relations schema version: {}",
            config.schema_version
        );
    }
    Ok(config)
}

pub fn save(config: &SkillRelationsConfig) -> Result<()> {
    validate(config)?;
    let path = relations_path()?;
    let parent = path
        .parent()
        .context("Skill relations path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create {}", parent.display()))?;
    let raw = serde_json::to_string_pretty(config)?;
    atomic_write::write_string_atomic(&path, &raw)
        .with_context(|| format!("Failed to write skill relations: {}", path.display()))
}

pub fn upsert_relation(relation: SkillRelationRule) -> Result<SkillRelationsConfig> {
    let mut config = load()?;
    if let Some(existing) = config
        .relations
        .iter_mut()
        .find(|item| item.id == relation.id)
    {
        *existing = relation;
    } else {
        config.relations.push(relation);
    }
    config
        .relations
        .sort_by(|left, right| left.id.cmp(&right.id));
    save(&config)?;
    Ok(config)
}

pub fn upsert_group(group: SkillGroup) -> Result<SkillRelationsConfig> {
    let mut config = load()?;
    if group.id.trim().is_empty() || group.name.trim().is_empty() {
        bail!("Skill group id and name cannot be empty");
    }
    if let Some(existing) = config.groups.iter_mut().find(|item| item.id == group.id) {
        *existing = group;
    } else {
        config.groups.push(group);
    }
    config.groups.sort_by(|left, right| left.id.cmp(&right.id));
    save(&config)?;
    Ok(config)
}
pub fn ignore_candidate(candidate: IgnoredSkillCandidate) -> Result<SkillRelationsConfig> {
    let mut config = load()?;
    config
        .ignored_candidates
        .retain(|item| item.candidate_key != candidate.candidate_key);
    config.ignored_candidates.push(candidate);
    config
        .ignored_candidates
        .sort_by(|left, right| left.candidate_key.cmp(&right.candidate_key));
    save(&config)?;
    Ok(config)
}
pub fn remove_relation(id: &str) -> Result<SkillRelationsConfig> {
    let mut config = load()?;
    let before = config.relations.len();
    config.relations.retain(|relation| relation.id != id);
    if config.relations.len() == before {
        bail!("Unknown skill relation: {id}");
    }
    save(&config)?;
    Ok(config)
}

fn validate(config: &SkillRelationsConfig) -> Result<()> {
    if config.schema_version != SKILL_RELATIONS_SCHEMA_VERSION {
        bail!("Skill relations schema version must be {SKILL_RELATIONS_SCHEMA_VERSION}");
    }
    let mut ids = std::collections::BTreeSet::new();
    for relation in &config.relations {
        if relation.id.trim().is_empty()
            || relation.from.skill_id.trim().is_empty()
            || relation.to.skill_id.trim().is_empty()
        {
            bail!("Skill relation ids and endpoints cannot be empty");
        }
        if relation.from.skill_id == relation.to.skill_id {
            bail!("A skill relation cannot target itself: {}", relation.id);
        }
        if !ids.insert(&relation.id) {
            bail!("Duplicate skill relation id: {}", relation.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::model::{
        relation_id, SkillRelationKind, SkillRelationSource, SkillSelector,
    };
    use tempfile::tempdir;

    #[test]
    fn relation_store_round_trips_and_rejects_self_edges() {
        let home = tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let from = SkillSelector {
            skill_id: "review".into(),
            source_provider: None,
            fingerprint: None,
        };
        let to = SkillSelector {
            skill_id: "testing".into(),
            source_provider: None,
            fingerprint: None,
        };
        let relation = SkillRelationRule {
            id: relation_id(&from, &SkillRelationKind::Orchestrates, &to),
            from,
            to,
            kind: SkillRelationKind::Orchestrates,
            source: SkillRelationSource::Manual,
            enabled: true,
            note: None,
            evidence: None,
        };
        upsert_relation(relation.clone()).unwrap();
        assert_eq!(load().unwrap().relations, vec![relation]);
        crate::config::reset_test_home_dir();
    }
}
