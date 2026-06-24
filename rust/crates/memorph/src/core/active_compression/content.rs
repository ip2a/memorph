#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectedContentKind {
    SearchResults,
    Diff,
    Log,
    JsonPayload,
    ConversationText,
}

impl DetectedContentKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SearchResults => "search_results",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::JsonPayload => "json_payload",
            Self::ConversationText => "conversation_text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContentProfile {
    pub kind: DetectedContentKind,
    pub bytes: usize,
    pub lines: usize,
    pub non_empty_lines: usize,
    pub search_match_lines: usize,
    pub diff_change_lines: usize,
    pub diagnostic_lines: usize,
}

impl ContentProfile {
    pub(super) fn describe(&self) -> String {
        format!(
            "kind={}, bytes={}, lines={}, non_empty_lines={}, search_matches={}, diff_changes={}, diagnostics={}",
            self.kind.as_str(),
            self.bytes,
            self.lines,
            self.non_empty_lines,
            self.search_match_lines,
            self.diff_change_lines,
            self.diagnostic_lines
        )
    }
}

pub(super) fn content_profile(text: &str) -> ContentProfile {
    let search_match_lines = text
        .lines()
        .filter(|line| parse_search_result_line(line).is_some())
        .count();
    let diff_change_lines = diff_change_line_count(text);
    let diagnostic_lines = diagnostic_line_count(text);
    ContentProfile {
        kind: detect_text_kind_from_counts(
            text,
            search_match_lines,
            diff_change_lines,
            diagnostic_lines,
        ),
        bytes: text.len(),
        lines: text.lines().count(),
        non_empty_lines: text.lines().filter(|line| !line.trim().is_empty()).count(),
        search_match_lines,
        diff_change_lines,
        diagnostic_lines,
    }
}

pub(super) fn detect_text_kind(text: &str) -> DetectedContentKind {
    detect_text_kind_from_counts(
        text,
        text.lines()
            .filter(|line| parse_search_result_line(line).is_some())
            .count(),
        diff_change_line_count(text),
        diagnostic_line_count(text),
    )
}

fn detect_text_kind_from_counts(
    text: &str,
    search_match_lines: usize,
    diff_change_lines: usize,
    diagnostic_lines: usize,
) -> DetectedContentKind {
    if search_match_lines >= 2 {
        return DetectedContentKind::SearchResults;
    }
    if looks_like_diff_with_change_count(text, diff_change_lines) {
        return DetectedContentKind::Diff;
    }
    if diagnostic_lines >= 2 {
        return DetectedContentKind::Log;
    }
    if looks_like_json_payload(text) {
        return DetectedContentKind::JsonPayload;
    }
    DetectedContentKind::ConversationText
}

pub(super) fn parse_search_result_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?.trim();
    let line_number = parts.next()?.trim();
    let text = parts.next()?.trim();
    if (path.contains('/') || path.contains('.')) && line_number.parse::<usize>().is_ok() {
        Some((path, line_number, text))
    } else {
        None
    }
}

fn looks_like_diff_with_change_count(text: &str, change_lines: usize) -> bool {
    let mut header_seen = false;
    for line in text.lines().take(120) {
        if line.starts_with("diff --git ") || line.starts_with("--- a/") || line.starts_with("@@ ")
        {
            header_seen = true;
        }
        if header_seen && change_lines >= 2 {
            return true;
        }
    }
    false
}

fn diff_change_line_count(text: &str) -> usize {
    text.lines()
        .take(120)
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++"))
                || (line.starts_with('-') && !line.starts_with("---"))
        })
        .count()
}

fn diagnostic_line_count(text: &str) -> usize {
    text.lines()
        .take(120)
        .filter(|line| is_diagnostic_line(line))
        .count()
}

fn is_diagnostic_line(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("warning")
        || lower.contains("warn")
        || lower.contains("failed")
        || lower.contains("failure")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.contains("traceback")
        || lower.starts_with("npm err!")
        || lower.starts_with("cargo ")
        || lower.starts_with("test result:")
        || trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Finished ")
        || trimmed.starts_with("Running ")
}

fn looks_like_json_payload(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return false;
    }
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_search_before_generic_log_text() {
        let text = "src/lib.rs:10:error match\nsrc/core.rs:20:warning match";

        assert_eq!(detect_text_kind(text), DetectedContentKind::SearchResults);
    }

    #[test]
    fn profiles_log_output() {
        let text = "Compiling memorph\nwarning: unused\nerror: failed\n";
        let profile = content_profile(text);

        assert_eq!(profile.kind, DetectedContentKind::Log);
        assert_eq!(profile.lines, 3);
        assert_eq!(profile.non_empty_lines, 3);
        assert_eq!(profile.diagnostic_lines, 3);
    }
}
