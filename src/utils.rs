// use std::path::Path;

/// Encode a directory path for Claude Code project naming
/// /Users/yuuu/work/2026_4/memorph -> -Users-yuuu-work-2026-4-memorph
pub fn encode_project_dir(path: &str) -> String {
    path.trim()
        .replace(['/', '\\'], "-")
        .replace(' ', "-")
        .replace('_', "-")
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
                    Some(format!("[Thinking: {}]", thinking.chars().take(100).collect::<String>()))
                } else if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    Some(format!("[Tool: {}]", name))
                } else {
                    None
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
