use crate::{
    canonical::EventBlock,
    core::{
        projection::list_sessions, sessions::get_session_detail_view, SessionDetailView,
        SessionHookFilter, SessionListParams, SessionListSort,
    },
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::server::SkillsOverview;

const SESSION_LIMIT: usize = 200;
const CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, Serialize)]
pub struct SkillUsage {
    pub skill_id: String,
    pub invocations: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub last_invoked_at: Option<String>,
    pub context_tokens: u64,
    pub context_budget_percent: f64,
    pub health_score: u8,
    pub prune_candidate: bool,
    pub reclaimable_tokens: u64,
    pub coverage_percent: f64,
    pub observed_files: Vec<String>,
    pub traces: Vec<SkillTrace>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillTriggerConflict {
    pub trigger: String,
    pub skills: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillTrace {
    pub provider_id: String,
    pub session_id: String,
    pub session_title: Option<String>,
    pub timestamp: String,
    pub event_id: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SkillUsageOverview {
    pub scanned_sessions: usize,
    pub failed_sessions: usize,
    pub invocations: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub trigger_conflicts: Vec<SkillTriggerConflict>,
    pub skills: Vec<SkillUsage>,
}

struct CachedAnalysis {
    created_at: Instant,
    skill_fingerprint: String,
    value: SkillUsageOverview,
}

static CACHE: OnceLock<Mutex<Option<CachedAnalysis>>> = OnceLock::new();

pub fn scan(overview: &SkillsOverview, refresh: bool) -> SkillUsageOverview {
    let fingerprint = overview
        .skills
        .iter()
        .map(|skill| format!("{}:{}", skill.id, skill.fingerprint))
        .collect::<Vec<_>>()
        .join("|");
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if !refresh {
        if let Ok(guard) = cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.created_at.elapsed() < CACHE_TTL
                    && cached.skill_fingerprint == fingerprint
                {
                    return cached.value.clone();
                }
            }
        }
    }

    let value = scan_uncached(overview);
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedAnalysis {
            created_at: Instant::now(),
            skill_fingerprint: fingerprint,
            value: value.clone(),
        });
    }
    value
}

fn scan_uncached(overview: &SkillsOverview) -> SkillUsageOverview {
    let mut result =
        SkillUsageOverview {
            skills: overview
                .skills
                .iter()
                .map(|skill| {
                    let context_tokens = skill.statistics.bytes.div_ceil(4);
                    let deductions =
                        (skill.issues.len() as u8).saturating_mul(10)
                            + u8::from(skill.conflict).saturating_mul(25)
                            + u8::from(skill.installations.iter().any(|installation| {
                                !installation.link_valid || installation.drifted
                            }))
                            .saturating_mul(20);
                    SkillUsage {
                        skill_id: skill.id.clone(),
                        context_tokens,
                        context_budget_percent: context_tokens as f64 / 2_000.0,
                        health_score: 100_u8.saturating_sub(deductions.min(100)),
                        reclaimable_tokens: context_tokens,
                        ..SkillUsage::default()
                    }
                })
                .collect(),
            trigger_conflicts: trigger_conflicts(overview),
            ..SkillUsageOverview::default()
        };
    let aliases = skill_aliases(overview);
    let auxiliary_files = auxiliary_files(overview);
    let Ok(groups) = list_sessions(&SessionListParams {
        all: true,
        providers: Vec::new(),
        cwd: None,
        include_message_counts: false,
        limit: Some(SESSION_LIMIT),
        offset: None,
        sort: SessionListSort::Recent,
        hook_filter: SessionHookFilter::All,
    }) else {
        return result;
    };

    for item in groups
        .into_iter()
        .flat_map(|group| {
            group
                .sessions
                .into_iter()
                .map(move |item| (group.provider_id.clone(), item))
        })
        .take(SESSION_LIMIT)
    {
        let (provider_id, session) = item;
        match get_session_detail_view(&provider_id, &session.session_id) {
            Ok(view) => {
                result.scanned_sessions += 1;
                analyze_session(&view, &aliases, &auxiliary_files, &mut result);
            }
            Err(_) => result.failed_sessions += 1,
        }
    }

    result.skills.sort_by(|left, right| {
        right
            .invocations
            .cmp(&left.invocations)
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });
    for usage in &mut result.skills {
        usage.prune_candidate = usage.invocations == 0;
        if !usage.prune_candidate {
            usage.reclaimable_tokens = 0;
        }
        let total_files = auxiliary_files
            .get(&usage.skill_id)
            .map_or(0, BTreeSet::len);
        usage.coverage_percent = if total_files == 0 {
            100.0
        } else {
            usage.observed_files.len() as f64 / total_files as f64 * 100.0
        };
    }
    result
}

fn skill_aliases(overview: &SkillsOverview) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    for skill in &overview.skills {
        for alias in [&skill.id, &skill.directory, &skill.name] {
            aliases.insert(normalize(alias), skill.id.clone());
        }
    }
    aliases
}

fn analyze_session(
    view: &SessionDetailView,
    aliases: &BTreeMap<String, String>,
    auxiliary_files: &BTreeMap<String, BTreeSet<String>>,
    result: &mut SkillUsageOverview,
) {
    let mut session_skills = BTreeSet::new();
    let session_content = serde_json::to_string(&view.events).unwrap_or_default();

    for event in &view.events {
        let mut invoked = BTreeSet::new();
        for block in &event.blocks {
            match block {
                EventBlock::ToolCall { name, input, .. } => {
                    if normalize(name) == "skill" {
                        if let Some(name) = input.as_ref().and_then(invoked_skill_name) {
                            invoked.insert(name);
                        }
                    } else {
                        invoked.insert(name.clone());
                    }
                }
                EventBlock::Text { text } => invoked.extend(command_invocations(text)),
                _ => {}
            }
        }

        for raw_name in invoked {
            let Some(skill_id) = aliases.get(&normalize(&raw_name)) else {
                continue;
            };
            let Some(usage) = result
                .skills
                .iter_mut()
                .find(|item| &item.skill_id == skill_id)
            else {
                continue;
            };
            usage.invocations += 1;
            session_skills.insert(skill_id.clone());
            let timestamp = event.timestamp.to_rfc3339();
            if usage
                .last_invoked_at
                .as_ref()
                .is_none_or(|last| timestamp > *last)
            {
                usage.last_invoked_at = Some(timestamp.clone());
            }
            if usage.traces.len() < 20 {
                usage.traces.push(SkillTrace {
                    provider_id: view.provider_id.clone(),
                    session_id: view.session_id.clone(),
                    session_title: view.display_title.clone().or_else(|| view.title.clone()),
                    timestamp,
                    event_id: event.id.clone(),
                    source: raw_name,
                });
            }
            if let Some(tokens) = event.metadata.usage.as_ref() {
                let input = tokens.input_tokens.unwrap_or(0);
                let output = tokens.output_tokens.unwrap_or(0);
                let total = tokens.total_tokens.unwrap_or(input + output);
                usage.input_tokens += input;
                usage.output_tokens += output;
                usage.total_tokens += total;
                result.input_tokens += input;
                result.output_tokens += output;
                result.total_tokens += total;
            }
            if let Some(cost) = event_cost(&event.metadata.provider_ext) {
                *usage.estimated_cost_usd.get_or_insert(0.0) += cost;
                *result.estimated_cost_usd.get_or_insert(0.0) += cost;
            }
            result.invocations += 1;
        }
    }

    for skill_id in session_skills {
        if let Some(usage) = result
            .skills
            .iter_mut()
            .find(|item| item.skill_id == skill_id)
        {
            usage.sessions += 1;
            if let Some(files) = auxiliary_files.get(&skill_id) {
                for file in files {
                    let basename = Path::new(file)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or(file);
                    if session_content.contains(file) || session_content.contains(basename) {
                        usage.observed_files.push(file.clone());
                    }
                }
                usage.observed_files.sort();
                usage.observed_files.dedup();
            }
        }
    }
}

fn auxiliary_files(overview: &SkillsOverview) -> BTreeMap<String, BTreeSet<String>> {
    let mut result = BTreeMap::new();
    for skill in &overview.skills {
        let Some(root) = skill.installations.first().map(|item| item.path.as_path()) else {
            continue;
        };
        let files = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(root).ok()?.to_string_lossy();
                (relative != "SKILL.md" && relative != ".memorph-managed-skill")
                    .then(|| relative.into_owned())
            })
            .collect();
        result.insert(skill.id.clone(), files);
    }
    result
}

fn trigger_conflicts(overview: &SkillsOverview) -> Vec<SkillTriggerConflict> {
    let mut triggers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for skill in &overview.skills {
        let Some(path) = skill
            .installations
            .first()
            .map(|item| item.path.join("SKILL.md"))
        else {
            continue;
        };
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let mut lines = content.lines();
        if lines.next().map(str::trim) != Some("---") {
            continue;
        }
        for line in lines.take_while(|line| line.trim() != "---") {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if !matches!(key.trim(), "trigger" | "triggers" | "command" | "commands") {
                continue;
            }
            for trigger in value
                .trim_matches([' ', '[', ']', '\'', '"'])
                .split(',')
                .map(normalize)
                .filter(|value| !value.is_empty())
            {
                triggers
                    .entry(trigger)
                    .or_default()
                    .insert(skill.id.clone());
            }
        }
    }
    triggers
        .into_iter()
        .filter(|(_, skills)| skills.len() > 1)
        .map(|(trigger, skills)| SkillTriggerConflict {
            trigger,
            skills: skills.into_iter().collect(),
        })
        .collect()
}

fn invoked_skill_name(input: &Value) -> Option<String> {
    ["skill", "name", "command"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(|value| value.trim_start_matches('/').to_string())
}

fn command_invocations(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for marker in ["<command-name>", "<skill-name>"] {
        let mut remainder = text;
        while let Some(start) = remainder.find(marker) {
            remainder = &remainder[start + marker.len()..];
            let end_tag = if marker == "<command-name>" {
                "</command-name>"
            } else {
                "</skill-name>"
            };
            let Some(end) = remainder.find(end_tag) else {
                break;
            };
            names.push(remainder[..end].trim().trim_start_matches('/').to_string());
            remainder = &remainder[end + end_tag.len()..];
        }
    }
    names
}

fn event_cost(values: &BTreeMap<String, Value>) -> Option<f64> {
    ["cost_usd", "costUSD", "cost"]
        .into_iter()
        .find_map(|key| values.get(key).and_then(Value::as_f64))
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('/')
        .to_lowercase()
        .replace(['_', ' '], "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tool_and_tag_invocations() {
        assert_eq!(
            invoked_skill_name(&serde_json::json!({"skill": "grill-me"})).as_deref(),
            Some("grill-me")
        );
        assert_eq!(
            command_invocations("<command-name>/review</command-name>"),
            vec!["review"]
        );
    }

    #[test]
    fn normalizes_skill_aliases() {
        assert_eq!(normalize("/Document Writer"), "document-writer");
        assert_eq!(normalize("document_writer"), "document-writer");
    }
}
