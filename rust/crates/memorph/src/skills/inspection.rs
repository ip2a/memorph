//! Skills 扫描核心层(类型 + bundle 检视 + frontmatter 读取)。
//!
//! 被 `skills::scanner`(扫描入口)与 `skills::server`(HTTP handler)共享。
//! 不依赖 axum,留在核心 crate。

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const MANAGED_MARKER: &str = ".memorph-managed-skill";
pub const MAX_ASSETS: usize = 200;
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_PREVIEW_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct SkillAsset {
    pub path: String,
    pub category: String,
    pub extension: Option<String>,
    pub bytes: u64,
    pub previewable: bool,
    pub entry: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillStatistics {
    pub files: usize,
    pub bytes: u64,
    pub scripts: usize,
    pub references: usize,
    pub assets: usize,
    pub previewable: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillIssue {
    pub path: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillAgent {
    pub provider_id: String,
    pub name: String,
    pub skills_dir: PathBuf,
    pub scope_kind: String,
    pub workspace_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillInstallation {
    pub provider_id: String,
    pub path: PathBuf,
    pub managed: bool,
    pub deployment_mode: String,
    pub link_valid: bool,
    pub fingerprint: String,
    pub drifted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub directory: String,
    pub fingerprint: String,
    pub conflict: bool,
    pub statistics: SkillStatistics,
    pub issues: Vec<SkillIssue>,
    pub installations: Vec<SkillInstallation>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub skill: SkillEntry,
    pub frontmatter: BTreeMap<String, String>,
    pub provider_metadata: Vec<SkillAsset>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillTree {
    pub skill_id: String,
    pub fingerprint: String,
    pub assets: Vec<SkillAsset>,
    pub issues: Vec<SkillIssue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillsOverview {
    pub agents: Vec<SkillAgent>,
    pub skills: Vec<SkillEntry>,
}

pub struct BundleInspection {
    pub fingerprint: String,
    pub statistics: SkillStatistics,
    pub assets: Vec<SkillAsset>,
    pub issues: Vec<SkillIssue>,
}

pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp"
    )
}

pub fn is_text_preview_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "markdown"
            | "txt"
            | "json"
            | "jsonc"
            | "yaml"
            | "yml"
            | "toml"
            | "js"
            | "ts"
            | "tsx"
            | "py"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "css"
            | "html"
            | "csv"
            | "ini"
    )
}

fn classify_asset(path: &str) -> (&'static str, bool) {
    let lower = path.to_ascii_lowercase();
    let category = if lower == "skill.md" {
        "entry"
    } else if lower.starts_with("scripts/") || lower.starts_with("script/") {
        "script"
    } else if lower.starts_with("references/") || lower.starts_with("reference/") {
        "reference"
    } else if lower.starts_with("assets/") || lower.starts_with("asset/") {
        "asset"
    } else if lower.starts_with("agents/") {
        "metadata"
    } else {
        "other"
    };
    let previewable = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_text_preview_extension(ext) || is_image_extension(ext));
    (category, previewable)
}

pub fn inspect_bundle(root: &Path) -> BundleInspection {
    let mut hasher = Sha256::new();
    let mut assets = Vec::new();
    let mut issues = Vec::new();
    let mut total_bytes = 0;
    let mut statistics = SkillStatistics {
        files: 0,
        bytes: 0,
        scripts: 0,
        references: 0,
        assets: 0,
        previewable: 0,
    };

    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let Ok(entry) = entry else {
            issues.push(SkillIssue {
                path: None,
                message: "Failed to read a bundle entry".into(),
            });
            continue;
        };
        let relative = match entry.path().strip_prefix(root) {
            Ok(value) => value.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if relative == MANAGED_MARKER || relative.starts_with(&format!("{MANAGED_MARKER}/")) {
            continue;
        }
        if entry.path_is_symlink() {
            issues.push(SkillIssue {
                path: Some(relative),
                message: "Symbolic links are not indexed".into(),
            });
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = fs::metadata(entry.path()) else {
            issues.push(SkillIssue {
                path: Some(relative),
                message: "File metadata is unreadable".into(),
            });
            continue;
        };
        let bytes = metadata.len();
        if assets.len() >= MAX_ASSETS || total_bytes + bytes > MAX_TOTAL_BYTES {
            issues.push(SkillIssue {
                path: Some(relative),
                message: "Asset index budget exceeded".into(),
            });
            continue;
        }
        let Ok(content) = fs::read(entry.path()) else {
            issues.push(SkillIssue {
                path: Some(relative),
                message: "File is unreadable".into(),
            });
            continue;
        };
        hasher.update(relative.as_bytes());
        hasher.update((bytes as u128).to_le_bytes());
        hasher.update(&content);
        let (category, previewable) = classify_asset(&relative);
        issues.extend(scan_content_risks(&relative, &content));
        let extension = Path::new(&relative)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        assets.push(SkillAsset {
            entry: relative == "SKILL.md",
            path: relative,
            category: category.to_string(),
            extension,
            bytes,
            previewable: previewable && bytes <= MAX_PREVIEW_BYTES,
        });
        total_bytes += bytes;
        statistics.files += 1;
        statistics.bytes += bytes;
        statistics.previewable += usize::from(previewable && bytes <= MAX_PREVIEW_BYTES);
        match category {
            "script" => statistics.scripts += 1,
            "reference" => statistics.references += 1,
            "asset" => statistics.assets += 1,
            _ => {}
        }
    }
    let frontmatter = read_frontmatter(&root.join("SKILL.md"));
    if !frontmatter.contains_key("name") {
        issues.push(SkillIssue {
            path: Some("SKILL.md".into()),
            message: "Quality signal: missing frontmatter name".into(),
        });
    }
    if !frontmatter.contains_key("description") {
        issues.push(SkillIssue {
            path: Some("SKILL.md".into()),
            message: "Quality signal: missing frontmatter description".into(),
        });
    }
    assets.sort_by(|left, right| left.path.cmp(&right.path));
    BundleInspection {
        fingerprint: format!("sha256:{:x}", hasher.finalize()),
        statistics,
        assets,
        issues,
    }
}

fn scan_content_risks(path: &str, content: &[u8]) -> Vec<SkillIssue> {
    let Ok(text) = std::str::from_utf8(content) else {
        return Vec::new();
    };
    let lower = text.to_ascii_lowercase();
    // ponytail: literal static signals only; add token-aware rules if false positives matter.
    [
        ("rm -rf", "Risk signal: recursive delete command"),
        ("curl", "Risk signal: curl network command"),
        ("wget", "Risk signal: wget network command"),
        ("sudo", "Risk signal: sudo privilege command"),
        ("~/.ssh", "Risk signal: SSH directory access"),
        ("os.environ", "Risk signal: environment variable access"),
        ("process.env", "Risk signal: environment variable access"),
    ]
    .into_iter()
    .filter(|(pattern, _)| lower.contains(pattern))
    .map(|(_, message)| SkillIssue {
        path: Some(path.to_string()),
        message: message.to_string(),
    })
    .collect()
}

pub fn read_frontmatter(path: &Path) -> BTreeMap<String, String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let mut result = BTreeMap::new();
    let mut lines = contents.lines();
    if lines.next().map(str::trim) != Some("---") {
        return result;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(['\'', '"']);
            if !key.trim().is_empty() && !value.is_empty() {
                result.insert(key.trim().to_string(), value.to_string());
            }
        }
    }
    result
}
