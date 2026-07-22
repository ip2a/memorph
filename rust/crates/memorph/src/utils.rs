// use std::path::Path;

/// Encode a directory path for Claude Code project naming
/// /Users/yuuu/work/2026_4/memorph -> -Users-yuuu-work-2026-4-memorph
pub fn encode_project_dir(path: &str) -> String {
    path.trim()
        .replace(['/', '\\'], "-")
        .replace([' ', '_'], "-")
}

/// Extract text from various content formats
pub fn extract_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    Some(text.to_string())
                } else if let Some(thinking) = item.get("thinking").and_then(|v| v.as_str()) {
                    Some(format!(
                        "[Thinking: {}]",
                        thinking.chars().take(100).collect::<String>()
                    ))
                } else {
                    item.get("name")
                        .and_then(|v| v.as_str())
                        .map(|name| format!("[Tool: {}]", name))
                }
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// Earliest plausible session timestamp: 2000-01-01 UTC.
pub const PLAUSIBLE_TIMESTAMP_MS: i64 = 946_684_800_000;

pub fn is_plausible_timestamp_ms(ms: i64) -> bool {
    ms >= PLAUSIBLE_TIMESTAMP_MS
}

pub fn datetime_from_timestamp_ms(ms: i64) -> Option<chrono::DateTime<chrono::Utc>> {
    is_plausible_timestamp_ms(ms)
        .then(|| chrono::DateTime::from_timestamp_millis(ms))
        .flatten()
}

pub fn is_plausible_session_time(dt: &chrono::DateTime<chrono::Utc>) -> bool {
    is_plausible_timestamp_ms(dt.timestamp_millis())
}

/// Parse RFC3339 or similar timestamp to milliseconds
pub fn parse_timestamp_to_ms(value: &serde_json::Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(if n > 1_000_000_000_000 { n } else { n * 1000 });
    }
    if let Some(n) = value.as_f64() {
        let n = n as i64;
        return Some(if n > 1_000_000_000_000 { n } else { n * 1000 });
    }
    let raw = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Truncate text for summary
pub fn truncate_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut result = trimmed.chars().take(max_chars).collect::<String>();
    result.push_str("...");
    result
}

/// Get basename from path string
pub fn path_basename(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.trim_end_matches(['/', '\\']);
    let last = normalized
        .split(['/', '\\'])
        .next_back()
        .filter(|segment| !segment.is_empty())?;
    Some(last.to_string())
}

/// Convert Windows extended-length paths into the standard form users expect.
pub fn user_visible_path(value: &str) -> String {
    if cfg!(windows) {
        return normalize_windows_user_visible_path(value);
    }

    value.to_string()
}

fn normalize_windows_user_visible_path(value: &str) -> String {
    const UNC_PREFIX: &str = r"\\?\UNC\";
    const VERBATIM_PREFIX: &str = r"\\?\";

    if value.len() >= UNC_PREFIX.len() && value[..UNC_PREFIX.len()].eq_ignore_ascii_case(UNC_PREFIX)
    {
        return format!(r"\\{}", &value[UNC_PREFIX.len()..]);
    }

    if let Some(stripped) = value.strip_prefix(VERBATIM_PREFIX) {
        return stripped.to_string();
    }

    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        datetime_from_timestamp_ms, is_plausible_timestamp_ms, normalize_windows_user_visible_path,
        PLAUSIBLE_TIMESTAMP_MS,
    };

    #[test]
    fn plausible_timestamp_ms_rejects_epoch_and_line_order_placeholders() {
        assert!(!is_plausible_timestamp_ms(0));
        assert!(!is_plausible_timestamp_ms(1));
        assert!(is_plausible_timestamp_ms(PLAUSIBLE_TIMESTAMP_MS));
        assert!(datetime_from_timestamp_ms(1_700_000_000_000).is_some());
        assert!(datetime_from_timestamp_ms(1).is_none());
    }

    #[test]
    fn windows_display_path_strips_verbatim_drive_prefix() {
        assert_eq!(
            normalize_windows_user_visible_path(r"\\?\D:\work\memorph"),
            r"D:\work\memorph"
        );
    }

    #[test]
    fn windows_display_path_strips_verbatim_unc_prefix() {
        assert_eq!(
            normalize_windows_user_visible_path(r"\\?\UNC\server\share\memorph"),
            r"\\server\share\memorph"
        );
    }

    #[test]
    fn windows_display_path_leaves_normal_path_unchanged() {
        assert_eq!(
            normalize_windows_user_visible_path(r"D:\work\memorph"),
            r"D:\work\memorph"
        );
    }
}
