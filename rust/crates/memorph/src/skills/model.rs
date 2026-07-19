use serde::{Deserialize, Serialize};

pub const SKILL_RELATIONS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SkillRelationKind {
    Requires,
    Uses,
    Orchestrates,
    RoutesTo,
    FallbackTo,
    Extends,
    MemberOf,
    RelatedTo,
    ConflictsWith,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSelector {
    pub skill_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SkillRelationSource {
    Manual,
    BundleMetadata,
    ConfirmedDetection,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRelationEvidence {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub excerpt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRelationRule {
    pub id: String,
    pub from: SkillSelector,
    pub to: SkillSelector,
    pub kind: SkillRelationKind,
    pub source: SkillRelationSource,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<SkillRelationEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGroup {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_skill: Option<SkillSelector>,
    #[serde(default)]
    pub members: Vec<SkillSelector>,
    pub source: SkillRelationSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnoredSkillCandidate {
    pub candidate_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRelationsConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub groups: Vec<SkillGroup>,
    #[serde(default)]
    pub relations: Vec<SkillRelationRule>,
    #[serde(default)]
    pub ignored_candidates: Vec<IgnoredSkillCandidate>,
}

impl Default for SkillRelationsConfig {
    fn default() -> Self {
        Self {
            schema_version: SKILL_RELATIONS_SCHEMA_VERSION,
            groups: Vec::new(),
            relations: Vec::new(),
            ignored_candidates: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_schema_version() -> u32 {
    SKILL_RELATIONS_SCHEMA_VERSION
}

pub fn relation_id(from: &SkillSelector, kind: &SkillRelationKind, to: &SkillSelector) -> String {
    format!(
        "{}:{}:{}",
        from.skill_id,
        relation_kind_name(kind),
        to.skill_id
    )
}

pub fn relation_kind_name(kind: &SkillRelationKind) -> &'static str {
    match kind {
        SkillRelationKind::Requires => "requires",
        SkillRelationKind::Uses => "uses",
        SkillRelationKind::Orchestrates => "orchestrates",
        SkillRelationKind::RoutesTo => "routes-to",
        SkillRelationKind::FallbackTo => "fallback-to",
        SkillRelationKind::Extends => "extends",
        SkillRelationKind::MemberOf => "member-of",
        SkillRelationKind::RelatedTo => "related-to",
        SkillRelationKind::ConflictsWith => "conflicts-with",
    }
}
