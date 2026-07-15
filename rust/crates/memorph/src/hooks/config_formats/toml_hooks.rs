//! Helpers for small TOML feature-flag edits used by hook installers.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::storage::atomic_write;

pub fn enable_bool_feature(path: &Path, key: &str) -> Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create hook config directory: {}",
                parent.display()
            )
        })?;
    }
    let original = fs::read_to_string(path).unwrap_or_default();
    let updated = ensure_features_bool_enabled(&original, key);
    if updated == original {
        return Ok(false);
    }
    atomic_write::write_string_atomic(path, &updated)?;
    Ok(true)
}

pub fn features_bool_enabled(contents: &str, key: &str) -> bool {
    let mut in_features = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && is_toml_bool_assignment(trimmed, key, true) {
            return true;
        }
    }
    false
}

fn ensure_features_bool_enabled(contents: &str, key: &str) -> String {
    let assignment = format!("{key} = true");
    let mut lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let had_trailing_newline = contents.ends_with('\n') || contents.is_empty();

    let mut features_start = None;
    let mut features_end = lines.len();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed == "[features]" {
                features_start = Some(idx);
                features_end = lines.len();
            } else if features_start.is_some() {
                features_end = idx;
                break;
            }
        }
    }

    if let Some(start) = features_start {
        for line in lines.iter_mut().take(features_end).skip(start + 1) {
            if toml_assignment_key(line.trim()) == Some(key) {
                if is_toml_bool_assignment(line.trim(), key, true) {
                    return join_lines(lines, had_trailing_newline);
                }
                *line = assignment;
                return join_lines(lines, true);
            }
        }
        lines.insert(start + 1, assignment);
        return join_lines(lines, true);
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push("[features]".to_string());
    lines.push(assignment);
    join_lines(lines, true)
}

fn is_toml_bool_assignment(line: &str, key: &str, expected: bool) -> bool {
    if toml_assignment_key(line) != Some(key) {
        return false;
    }
    line.split_once('=')
        .map(|(_, value)| {
            value
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case(if expected { "true" } else { "false" })
        })
        .unwrap_or(false)
}

fn toml_assignment_key(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    trimmed.split_once('=').map(|(key, _)| key.trim())
}

fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut output = lines.join("\n");
    if trailing_newline && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_flag_reader_detects_enabled_bool() {
        assert!(features_bool_enabled("[features]\nhooks = true\n", "hooks"));
        assert!(!features_bool_enabled(
            "[features]\nhooks = false\n",
            "hooks"
        ));
        assert!(!features_bool_enabled("[other]\nhooks = true\n", "hooks"));
    }
}
