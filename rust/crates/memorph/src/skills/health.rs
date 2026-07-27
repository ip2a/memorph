use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

use super::context;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HealthCheck {
    pub check_id: String,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub evidence: String,
    pub recommendation: String,
    pub checked_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillHealth {
    pub skill_id: String,
    pub status: String,
    pub score: u8,
    pub checks: Vec<HealthCheck>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HealthSummary {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub healthy: usize,
    pub completeness_status: String,
    pub skills: Vec<SkillHealth>,
}

pub fn summary(conn: &Connection) -> Result<HealthSummary> {
    let mut statement = conn.prepare("SELECT id FROM skill_catalog WHERE missing_since_ms IS NULL ORDER BY canonical_name COLLATE NOCASE")?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let skills = ids
        .into_iter()
        .map(|id| detail(conn, &id))
        .collect::<Result<Vec<_>>>()?;
    let completeness_status = conn.query_row("SELECT completeness_status FROM skill_scan_state WHERE state_key = 'skill-sessions:all'", [], |row| row.get(0)).unwrap_or_else(|_| "unknown".into());
    Ok(HealthSummary {
        total: skills.len(),
        errors: skills.iter().filter(|item| item.status == "error").count(),
        warnings: skills
            .iter()
            .filter(|item| item.status == "warning")
            .count(),
        healthy: skills.iter().filter(|item| item.status == "pass").count(),
        completeness_status,
        skills,
    })
}

pub fn detail(conn: &Connection, skill_id: &str) -> Result<SkillHealth> {
    let (name, description, metadata_json, manifest_json, file_count, total_bytes, path, install_kind, link_status, marker): (String, Option<String>, String, String, i64, i64, String, String, String, bool) = conn.query_row(
        "SELECT c.canonical_name, c.description, c.metadata_json, c.file_manifest_json, c.file_count, c.total_bytes,
                i.install_path, i.install_kind, i.link_status, i.managed_marker_present
         FROM skill_catalog c JOIN skill_installations i ON i.id = (
           SELECT id FROM skill_installations WHERE skill_id = c.id AND status = 'active' ORDER BY provider_id, id LIMIT 1)
         WHERE c.id = ?1", [skill_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
    ).context("Skill not found")?;
    let now = Utc::now().timestamp_millis();
    let root = Path::new(&path);
    let entry = root.join("SKILL.md");
    let mut checks = Vec::new();
    checks.push(check(
        "entry.readable",
        "entry",
        if entry.is_file() { "pass" } else { "error" },
        "Skill 入口文件",
        format!(
            "{} {}",
            entry.display(),
            if entry.is_file() {
                "可读取"
            } else {
                "不存在"
            }
        ),
        "确保安装目录包含可读取的 SKILL.md",
        now,
    ));
    checks.push(check(
        "metadata.name",
        "metadata",
        if name.trim().is_empty() {
            "error"
        } else {
            "pass"
        },
        "名称元数据",
        format!("name={name}"),
        "在 frontmatter 中填写稳定名称",
        now,
    ));
    checks.push(check(
        "metadata.description",
        "metadata",
        if description.as_deref().unwrap_or("").trim().is_empty() {
            "warning"
        } else {
            "pass"
        },
        "描述元数据",
        description
            .clone()
            .unwrap_or_else(|| "description 缺失".into()),
        "补充简洁、可区分的 description",
        now,
    ));
    let metadata_bytes = metadata_json.len();
    checks.push(check(
        "metadata.length",
        "metadata",
        if metadata_bytes > 4_096 {
            "warning"
        } else {
            "pass"
        },
        "元数据长度",
        format!("metadata_bytes={metadata_bytes}; limit=4096"),
        "精简 frontmatter，只保留识别 Skill 所需字段",
        now,
    ));
    checks.push(check(
        "bundle.nonempty",
        "bundle",
        if file_count == 0 { "warning" } else { "pass" },
        "Bundle 内容",
        format!("{file_count} files, {total_bytes} bytes"),
        "移除空 Skill 或补充所需文件",
        now,
    ));
    checks.push(check(
        "bundle.size",
        "bundle",
        if total_bytes > 2 * 1024 * 1024 || file_count > 200 {
            "warning"
        } else {
            "pass"
        },
        "Bundle 规模",
        format!("{file_count} files, {total_bytes} bytes; limits=200 files/2 MiB"),
        "将大型参考资料移出 Skill 或按需拆分",
        now,
    ));
    let manifest =
        serde_json::from_str::<Vec<serde_json::Value>>(&manifest_json).unwrap_or_default();
    let paths = manifest
        .iter()
        .filter_map(|item| item.get("path").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    let unique_paths = paths.iter().copied().collect::<BTreeSet<_>>();
    let largest_file = manifest
        .iter()
        .filter_map(|item| item.get("bytes").and_then(|value| value.as_u64()))
        .max()
        .unwrap_or(0);
    checks.push(check(
        "bundle.manifest",
        "bundle",
        if paths.len() != unique_paths.len() || largest_file > 512 * 1024 {
            "warning"
        } else {
            "pass"
        },
        "Bundle 文件清单",
        format!(
            "duplicate_paths={}, largest_file_bytes={largest_file}; limit=524288",
            paths.len().saturating_sub(unique_paths.len())
        ),
        "移除重复路径，并将超大文件移出常用 Bundle",
        now,
    ));

    let entry_text = fs::read_to_string(&entry).unwrap_or_default();
    let body_name = entry_text
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(str::trim);
    let names_match = body_name.is_none_or(|body| normalize(body) == normalize(&name));
    checks.push(check(
        "consistency.name",
        "consistency",
        if names_match { "pass" } else { "warning" },
        "名称一致性",
        format!("metadata={name:?}, first_heading={body_name:?}"),
        "使 frontmatter 名称与正文一级标题保持一致",
        now,
    ));

    let (broken_links, outside_links, link_cycle) = inspect_links(root);
    checks.push(check(
        "link.local-targets",
        "link",
        if outside_links > 0 {
            "error"
        } else if broken_links > 0 || link_cycle {
            "warning"
        } else {
            "pass"
        },
        "本地链接",
        format!("broken={broken_links}, outside={outside_links}, cycle={link_cycle}"),
        "修复断链和循环引用，并确保相对链接不越出 Skill 根目录",
        now,
    ));

    checks.push(check(
        "deployment.link",
        "deployment",
        if link_status == "broken" {
            "error"
        } else {
            "pass"
        },
        "安装链接状态",
        format!("kind={install_kind}, link={link_status}"),
        "重新部署失效符号链接",
        now,
    ));
    checks.push(check(
        "deployment.marker",
        "deployment",
        if install_kind == "managed-copy" && !marker {
            "error"
        } else {
            "pass"
        },
        "受管复制标记",
        format!("managed_marker_present={marker}"),
        "不要自动删除缺少受管标记的目录；重新部署后再清理",
        now,
    ));
    if let Ok(value) = context::detail(conn, skill_id, None) {
        let tokens = value.metadata.estimated_tokens + value.body.estimated_tokens;
        checks.push(check(
            "context.resident",
            "context",
            if tokens > 8_000 { "warning" } else { "pass" },
            "上下文预算",
            format!(
                "metadata+body≈{tokens} tokens; algorithm={}",
                context::ALGORITHM_VERSION
            ),
            "精简常驻元数据或将大段参考内容移到按需文件",
            now,
        ));
    }
    let risky = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|item| item.file_type().is_file())
        .filter_map(|item| {
            fs::read_to_string(item.path())
                .ok()
                .map(|text| (item.path().display().to_string(), text.to_lowercase()))
        })
        .find_map(|(path, content)| {
            ["rm -rf", "sudo ", "curl ", "wget "]
                .into_iter()
                .find(|pattern| content.contains(pattern))
                .map(|pattern| format!("{path}: {pattern}"))
        });
    checks.push(check(
        "security.static-patterns",
        "security",
        if risky.is_some() { "warning" } else { "pass" },
        "危险命令静态检查",
        risky
            .map(|value| format!("匹配模式: {value}"))
            .unwrap_or_else(|| "未发现内置危险模式".into()),
        "人工审查命令；Memorph 不执行 Skill 内容",
        now,
    ));
    let completeness = conn
        .query_row(
            "SELECT completeness_status FROM skill_scan_state WHERE state_key = 'skill-sessions:all'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "unknown".into());
    checks.push(check(
        "database.invocation-index",
        "database",
        if completeness == "error" {
            "error"
        } else if completeness == "complete" {
            "pass"
        } else {
            "info"
        },
        "会话索引完整性",
        format!("completeness={completeness}"),
        "完成会话索引后再判断长期未使用",
        now,
    ));
    let last_invoked = conn
        .query_row(
            "SELECT MAX(invoked_at_ms) FROM skill_invocations WHERE skill_id = ?1",
            [skill_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None);
    let unused = completeness == "complete"
        && last_invoked.is_none_or(|value| value < now - 90 * 86_400_000);
    checks.push(check(
        "usage.recent",
        "usage",
        if unused {
            "info"
        } else if completeness == "complete" {
            "pass"
        } else {
            "info"
        },
        "近期使用",
        format!("last_invoked_at_ms={last_invoked:?}, completeness={completeness}"),
        "仅在完整历史窗口下考虑清理长期未使用 Skill",
        now,
    ));
    let coverage = conn
        .query_row(
            "SELECT COUNT(*) FROM skill_coverage_observations WHERE skill_id = ?1",
            [skill_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    checks.push(check(
        "coverage.observations",
        "coverage",
        if coverage > 0 { "pass" } else { "info" },
        "覆盖证据",
        format!("observations={coverage}"),
        "运行会话索引以收集文件和章节覆盖证据",
        now,
    ));
    let conflicting_names = conn
        .query_row(
            "SELECT COUNT(*) FROM skill_catalog WHERE id <> ?1 AND normalized_name = (SELECT normalized_name FROM skill_catalog WHERE id = ?1) AND missing_since_ms IS NULL",
            [skill_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    checks.push(check(
        "conflict.normalized-name",
        "conflict",
        if conflicting_names > 0 {
            "warning"
        } else {
            "pass"
        },
        "名称冲突",
        format!("same_normalized_name={conflicting_names}"),
        "调整名称或描述以避免触发歧义",
        now,
    ));
    let metadata_valid = serde_json::from_str::<BTreeMap<String, String>>(&metadata_json).is_ok();
    checks.push(check(
        "metadata.format",
        "metadata",
        if metadata_valid { "pass" } else { "error" },
        "元数据格式",
        format!("metadata_json_valid={metadata_valid}"),
        "修复 SKILL.md frontmatter 格式",
        now,
    ));
    let score = score(&checks);
    let status = if checks.iter().any(|item| item.severity == "error") {
        "error"
    } else if checks.iter().any(|item| item.severity == "warning") {
        "warning"
    } else {
        "pass"
    }
    .into();
    Ok(SkillHealth {
        skill_id: skill_id.into(),
        status,
        score,
        checks,
    })
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase().replace(['_', ' '], "-")
}

fn inspect_links(root: &Path) -> (usize, usize, bool) {
    let mut graph = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
    let mut broken = 0;
    let mut outside = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
    {
        let source = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_path_buf();
        let text = fs::read_to_string(entry.path()).unwrap_or_default();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("](") {
            rest = &rest[start + 2..];
            let Some(end) = rest.find(')') else { break };
            let target = rest[..end].trim().trim_matches(['<', '>']);
            rest = &rest[end + 1..];
            let target = target.split('#').next().unwrap_or("").trim();
            if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let path = Path::new(target);
            let mut relative = PathBuf::new();
            let mut escaped = path.is_absolute();
            for component in source
                .parent()
                .unwrap_or(Path::new(""))
                .join(path)
                .components()
            {
                match component {
                    Component::Normal(value) => relative.push(value),
                    Component::ParentDir if !relative.pop() => escaped = true,
                    Component::RootDir | Component::Prefix(_) => escaped = true,
                    Component::CurDir | Component::ParentDir => {}
                }
            }
            if escaped {
                outside += 1;
                continue;
            }
            let target_path = root.join(&relative);
            if !target_path.exists() {
                broken += 1;
            } else if target_path.canonicalize().is_ok_and(|path| {
                !path.starts_with(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
            }) {
                outside += 1;
            } else if relative.extension().is_some_and(|ext| ext == "md") {
                graph.entry(source.clone()).or_default().push(relative);
            }
        }
    }
    let cycle = graph.keys().any(|start| {
        let mut pending = graph.get(start).cloned().unwrap_or_default();
        let mut seen = BTreeSet::new();
        while let Some(path) = pending.pop() {
            if &path == start {
                return true;
            }
            if seen.insert(path.clone()) {
                pending.extend(graph.get(&path).cloned().unwrap_or_default());
            }
        }
        false
    });
    (broken, outside, cycle)
}

fn check(
    id: &str,
    category: &str,
    severity: &str,
    title: &str,
    evidence: String,
    recommendation: &str,
    checked_at_ms: i64,
) -> HealthCheck {
    HealthCheck {
        check_id: id.into(),
        category: category.into(),
        severity: severity.into(),
        title: title.into(),
        description: title.into(),
        evidence,
        recommendation: recommendation.into(),
        checked_at_ms,
    }
}

fn score(checks: &[HealthCheck]) -> u8 {
    let mut categories = BTreeMap::<&str, u8>::new();
    for item in checks {
        let deduction = match item.severity.as_str() {
            "error" => 20,
            "warning" => 8,
            _ => 0,
        };
        let value = categories.entry(&item.category).or_default();
        *value = (*value + deduction).min(40);
    }
    100_u8.saturating_sub(categories.values().copied().sum::<u8>())
}

#[cfg(test)]
mod tests {
    use super::{inspect_links, score, HealthCheck};
    use std::fs;

    #[test]
    fn local_links_report_broken_outside_and_cycles() {
        let root = std::env::temp_dir().join(format!("memorph-health-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "[a](docs/a.md) [missing](missing.md) [outside](../x.md)",
        )
        .unwrap();
        fs::write(root.join("docs/a.md"), "[back](../SKILL.md)").unwrap();
        assert_eq!(inspect_links(&root), (1, 1, true));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn score_caps_each_category_at_forty() {
        let item = |severity: &str| HealthCheck {
            check_id: "x".into(),
            category: "entry".into(),
            severity: severity.into(),
            title: "x".into(),
            description: "x".into(),
            evidence: "x".into(),
            recommendation: "x".into(),
            checked_at_ms: 0,
        };
        assert_eq!(
            score(&[item("error"), item("error"), item("error"), item("warning")]),
            60
        );
    }
}
