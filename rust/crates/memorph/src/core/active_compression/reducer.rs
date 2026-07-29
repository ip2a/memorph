use std::collections::{BTreeMap, BTreeSet};

use crate::provider::canonical_event_text;
use crate::session::{Block, Event, Role};

use super::adaptive::adaptive_keep_count;
use super::content::{content_profile, detect_text_kind, parse_search_result_line};
use super::{CompressionCandidateKind, CompressionCandidateReport};

pub(super) fn reduce_candidate_to_summary(
    candidate: &CompressionCandidateReport,
    source_events: &[Event],
    archive_ref: Option<&str>,
) -> String {
    let mut lines = vec![
        "[Compressed session segment]".to_string(),
        format!("Compression kind: {:?}", candidate.kind),
        format!("Selection reason: {:?}", candidate.reason),
        format!("Source events: {}", source_events.len()),
        format!("Source event ids: {}", candidate.event_ids.join(", ")),
        format!(
            "Original size: {} bytes / {} estimated tokens",
            candidate.original_estimated_bytes, candidate.original_estimated_tokens
        ),
        format!(
            "Estimated retained size: {} bytes / {} estimated tokens",
            candidate.compressed_estimated_bytes, candidate.compressed_estimated_tokens
        ),
        format!(
            "Rule strategy: {}",
            rule_strategy(candidate.kind, source_events)
        ),
        format!("Content profile: {}", source_content_profile(source_events)),
        "Retention policy: keep structural metadata, high-signal anchors, and archive-backed recovery; omit low-signal bulk text from the active context.".to_string(),
    ];

    let signals = compression_signals(candidate.kind, source_events);
    if !signals.is_empty() {
        lines.push("Retained signals:".to_string());
        lines.extend(signals.into_iter().map(|signal| format!("- {}", signal)));
    }

    if let Some(archive_ref) = archive_ref {
        lines.push(format!("Recovery archive: {}", archive_ref));
    } else {
        lines.push("Recovery archive: pending".to_string());
    }
    lines.join("\n")
}

fn rule_strategy(kind: CompressionCandidateKind, source_events: &[Event]) -> String {
    match kind {
        CompressionCandidateKind::LargeToolOutput => {
            "tool-output reducer: preserve tool ids, error state, detected content kind, and bounded high-signal lines"
                .to_string()
        }
        CompressionCandidateKind::LargeLogOutput => {
            "log reducer: classify lines, keep errors/failures/warnings/stack/summary/tail with adaptive caps"
                .to_string()
        }
        CompressionCandidateKind::LargeDiffOutput => {
            "diff reducer: parse files and hunks, keep changed-file anchors, adaptive hunk samples, and representative changes"
                .to_string()
        }
        CompressionCandidateKind::SearchResults => {
            "search-results reducer: group by file, keep first/last and scored matches with adaptive per-file caps"
                .to_string()
        }
        CompressionCandidateKind::ProviderPayloadText => {
            "provider-payload reducer: preserve payload kind, top-level schema keys, and byte scale"
                .to_string()
        }
        CompressionCandidateKind::HistoricalConversationRange => format!(
            "conversation-range reducer: preserve role counts, first user anchor, last assistant anchor, and file references ({})",
            detected_content_profile(source_events)
        ),
    }
}

fn source_content_profile(source_events: &[Event]) -> String {
    content_profile(&joined_event_text(source_events)).describe()
}

fn detected_content_profile(source_events: &[Event]) -> &'static str {
    detect_text_kind(&joined_event_text(source_events)).as_str()
}

fn joined_event_text(source_events: &[Event]) -> String {
    source_events
        .iter()
        .map(canonical_event_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn compression_signals(kind: CompressionCandidateKind, source_events: &[Event]) -> Vec<String> {
    match kind {
        CompressionCandidateKind::LargeToolOutput => tool_output_signals(source_events),
        CompressionCandidateKind::LargeLogOutput => command_log_signals(source_events),
        CompressionCandidateKind::LargeDiffOutput => diff_signals(source_events),
        CompressionCandidateKind::SearchResults => search_result_signals(source_events),
        CompressionCandidateKind::ProviderPayloadText => provider_payload_signals(source_events),
        CompressionCandidateKind::HistoricalConversationRange => {
            conversation_signals(source_events)
        }
    }
}

fn tool_output_signals(source_events: &[Event]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut result_count = 0usize;
    let mut error_count = 0usize;
    let mut call_ids = Vec::new();
    let mut highlights = Vec::new();

    for block in source_events.iter().flat_map(|event| event.blocks.iter()) {
        let Block::ToolResult {
            tool_call_id,
            content,
            is_error,
        } = block
        else {
            continue;
        };
        result_count += 1;
        if *is_error {
            error_count += 1;
        }
        push_unique(&mut call_ids, tool_call_id);
        collect_highlight_lines(content, &mut highlights);
    }

    push_signal(&mut signals, format!("Tool results: {}", result_count));
    if error_count > 0 {
        push_signal(&mut signals, format!("Tool errors: {}", error_count));
    }
    if !call_ids.is_empty() {
        push_signal(
            &mut signals,
            format!("Tool call ids: {}", call_ids.join(", ")),
        );
    }
    append_labeled_items(&mut signals, "Highlight", highlights);
    signals
}

#[derive(Clone)]
struct SearchMatch {
    file: String,
    line_number: usize,
    content: String,
    score: i32,
    order: usize,
}

fn search_result_signals(source_events: &[Event]) -> Vec<String> {
    let matches = parse_search_matches(source_events);
    if matches.is_empty() {
        return tool_output_signals(source_events);
    }

    let total_matches = matches.len();
    let mut grouped = BTreeMap::<String, Vec<SearchMatch>>::new();
    for item in matches {
        grouped.entry(item.file.clone()).or_default().push(item);
    }

    let mut files = grouped.into_iter().collect::<Vec<_>>();
    files.sort_by(|(left_file, left_matches), (right_file, right_matches)| {
        let left_score = left_matches.iter().map(|item| item.score).sum::<i32>();
        let right_score = right_matches.iter().map(|item| item.score).sum::<i32>();
        right_score
            .cmp(&left_score)
            .then_with(|| right_matches.len().cmp(&left_matches.len()))
            .then_with(|| left_file.cmp(right_file))
    });

    let file_items = files
        .iter()
        .map(|(file, matches)| format!("{} {}", file, matches.len()))
        .collect::<Vec<_>>();
    let keep_file_count = adaptive_keep_count(&file_items, 3, 12).max(1);
    let total_files = files.len();
    let omitted_files = total_files.saturating_sub(keep_file_count);

    let mut signals = Vec::new();
    let mut file_summaries = Vec::new();
    let mut retained_matches = Vec::new();
    let mut kept_matches = 0usize;
    let mut omitted_matches = 0usize;

    for (file, file_matches) in files.into_iter().take(keep_file_count) {
        let selected = select_search_matches(&file_matches);
        kept_matches += selected.len();
        omitted_matches += file_matches.len().saturating_sub(selected.len());
        file_summaries.push(format!(
            "{} ({}/{})",
            file,
            selected.len(),
            file_matches.len()
        ));
        for item in selected {
            retained_matches.push(format!(
                "{}:{} {}",
                item.file,
                item.line_number,
                truncate_preview(&item.content, 120)
            ));
        }
    }

    omitted_matches += file_items
        .iter()
        .skip(keep_file_count)
        .filter_map(|item| item.rsplit_once(' '))
        .filter_map(|(_, count)| count.parse::<usize>().ok())
        .sum::<usize>();

    push_signal(
        &mut signals,
        format!(
            "Search matches: total={}, kept={}, omitted={}, files={}, kept_files={}, omitted_files={}",
            total_matches, kept_matches, omitted_matches, total_files, keep_file_count, omitted_files
        ),
    );
    if !file_summaries.is_empty() {
        push_signal(
            &mut signals,
            format!("Matched files: {}", file_summaries.join(", ")),
        );
    }
    append_labeled_items(&mut signals, "Match", retained_matches);
    if omitted_matches > 0 {
        push_signal(
            &mut signals,
            format!("Omitted search matches: {}", omitted_matches),
        );
    }
    signals
}

fn parse_search_matches(source_events: &[Event]) -> Vec<SearchMatch> {
    let mut matches = Vec::new();
    for event in source_events {
        let event_text = canonical_event_text(event);
        for line in event_text.lines() {
            let Some((path, line_number, text)) = parse_search_result_line(line) else {
                continue;
            };
            let line_number = line_number.parse::<usize>().unwrap_or_default();
            matches.push(SearchMatch {
                file: path.to_string(),
                line_number,
                content: text.to_string(),
                score: score_search_match(text),
                order: matches.len(),
            });
        }
    }
    matches
}

fn select_search_matches(matches: &[SearchMatch]) -> Vec<SearchMatch> {
    if matches.is_empty() {
        return Vec::new();
    }

    let items = matches
        .iter()
        .map(|item| item.content.clone())
        .collect::<Vec<_>>();
    let keep_count = adaptive_keep_count(&items, 2, 5).max(1);
    let mut selected = BTreeSet::new();
    selected.insert(0usize);
    selected.insert(matches.len() - 1);

    let mut ranked = (0..matches.len()).collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        matches[*right]
            .score
            .cmp(&matches[*left].score)
            .then_with(|| matches[*left].order.cmp(&matches[*right].order))
    });
    for index in ranked {
        if selected.len() >= keep_count {
            break;
        }
        selected.insert(index);
    }

    selected
        .into_iter()
        .map(|index| matches[index].clone())
        .collect()
}

fn score_search_match(text: &str) -> i32 {
    let lower = text.to_ascii_lowercase();
    let mut score = 10 + (text.len() / 48).min(8) as i32;
    if lower.contains("error")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("fail")
        || lower.contains("exception")
    {
        score += 100;
    }
    if lower.contains("warn") {
        score += 70;
    }
    if lower.contains("todo") || lower.contains("fixme") {
        score += 35;
    }
    if lower.contains("fn ")
        || lower.contains("class ")
        || lower.contains("struct ")
        || lower.contains("impl ")
        || lower.contains("function ")
    {
        score += 30;
    }
    score
}

#[derive(Clone)]
struct LogRecord {
    line_number: usize,
    content: String,
    level: LogLevel,
    is_stack_trace: bool,
    is_summary: bool,
    score: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    Error,
    Fail,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

fn command_log_signals(source_events: &[Event]) -> Vec<String> {
    let (records, commands, exit_codes) = collect_log_records(source_events);
    if records.is_empty() {
        return Vec::new();
    }

    let selected = select_log_records(&records);
    let mut signals = Vec::new();
    let error_count = records
        .iter()
        .filter(|record| matches!(record.level, LogLevel::Error | LogLevel::Fail))
        .count();
    let warning_count = records
        .iter()
        .filter(|record| record.level == LogLevel::Warn)
        .count();
    let stack_count = records
        .iter()
        .filter(|record| record.is_stack_trace)
        .count();
    let summary_count = records.iter().filter(|record| record.is_summary).count();

    if !commands.is_empty() {
        push_signal(&mut signals, format!("Commands: {}", commands.join(" | ")));
    }
    if !exit_codes.is_empty() {
        push_signal(
            &mut signals,
            format!("Exit codes: {}", exit_codes.join(", ")),
        );
    }
    push_signal(
        &mut signals,
        format!(
            "Log lines: total={}, kept={}, omitted={}, errors={}, warnings={}, stack={}, summary={}",
            records.len(),
            selected.len(),
            records.len().saturating_sub(selected.len()),
            error_count,
            warning_count,
            stack_count,
            summary_count
        ),
    );

    append_labeled_items(
        &mut signals,
        "Log signal",
        selected
            .into_iter()
            .map(|record| {
                format!(
                    "{}:{}",
                    record.line_number,
                    truncate_preview(record.content.trim(), 140)
                )
            })
            .collect(),
    );
    signals
}

fn collect_log_records(source_events: &[Event]) -> (Vec<LogRecord>, Vec<String>, Vec<String>) {
    let mut raw_lines = Vec::new();
    let mut commands = Vec::new();
    let mut exit_codes = Vec::new();
    let mut saw_command_result = false;

    for block in source_events.iter().flat_map(|event| event.blocks.iter()) {
        let Block::CommandResult {
            command,
            exit_code,
            stdout,
            stderr,
        } = block
        else {
            continue;
        };
        saw_command_result = true;
        if let Some(command) = command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            push_unique(&mut commands, command);
        }
        if let Some(exit_code) = exit_code {
            push_unique_owned(&mut exit_codes, exit_code.to_string());
        }
        if let Some(stderr) = stderr {
            raw_lines.extend(stderr.lines().map(str::to_string));
        }
        if let Some(stdout) = stdout {
            raw_lines.extend(stdout.lines().map(str::to_string));
        }
    }

    if !saw_command_result {
        raw_lines.extend(joined_event_text(source_events).lines().map(str::to_string));
    }

    let records = raw_lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, content)| {
            let trimmed = content.trim_end();
            (!trimmed.trim().is_empty()).then(|| classify_log_line(index + 1, trimmed))
        })
        .collect::<Vec<_>>();

    (records, commands, exit_codes)
}

fn classify_log_line(line_number: usize, content: &str) -> LogRecord {
    let trimmed = content.trim();
    let lower = trimmed.to_ascii_lowercase();
    let is_stack_trace = lower.contains("traceback")
        || lower.starts_with("at ")
        || trimmed.starts_with("File \"")
        || lower.starts_with("caused by:")
        || lower.starts_with("stack backtrace:");
    let is_summary = lower.starts_with("test result:")
        || lower.starts_with("finished ")
        || lower.starts_with("running ")
        || lower.starts_with("collected ")
        || lower.starts_with("failures:")
        || lower.starts_with("errors:")
        || trimmed.starts_with("===")
        || trimmed.starts_with("---");
    let level = if lower.contains("fatal")
        || lower.contains("critical")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.contains(" error")
        || lower.starts_with("error")
        || lower.starts_with("npm err!")
    {
        LogLevel::Error
    } else if lower.contains("failed") || lower.contains("failure") || lower.starts_with("fail") {
        LogLevel::Fail
    } else if lower.contains("warning") || lower.contains("warn") {
        LogLevel::Warn
    } else if lower.contains("debug") {
        LogLevel::Debug
    } else if lower.contains("trace") || is_stack_trace {
        LogLevel::Trace
    } else if lower.contains("info") {
        LogLevel::Info
    } else {
        LogLevel::Unknown
    };

    let mut score = match level {
        LogLevel::Error => 100,
        LogLevel::Fail => 95,
        LogLevel::Warn => 70,
        LogLevel::Trace => 55,
        LogLevel::Info => 20,
        LogLevel::Debug => 5,
        LogLevel::Unknown => 10,
    };
    if is_stack_trace {
        score += 25;
    }
    if is_summary {
        score += 35;
    }

    LogRecord {
        line_number,
        content: trimmed.to_string(),
        level,
        is_stack_trace,
        is_summary,
        score,
    }
}

fn select_log_records(records: &[LogRecord]) -> Vec<LogRecord> {
    let mut selected = BTreeSet::new();

    for (index, record) in records.iter().enumerate() {
        if matches!(record.level, LogLevel::Error | LogLevel::Fail) {
            selected.insert(index);
            if index > 0 {
                selected.insert(index - 1);
            }
            if index + 1 < records.len() {
                selected.insert(index + 1);
            }
        }
    }

    let mut warning_keys = BTreeSet::new();
    for (index, record) in records.iter().enumerate() {
        if record.level != LogLevel::Warn {
            continue;
        }
        let key = normalize_variable_tail(&record.content);
        if warning_keys.insert(key) {
            selected.insert(index);
        }
        if warning_keys.len() >= 8 {
            break;
        }
    }

    for (index, record) in records.iter().enumerate() {
        if record.is_stack_trace || record.is_summary {
            selected.insert(index);
        }
        if selected.len() >= 80 {
            break;
        }
    }

    for index in records.len().saturating_sub(3)..records.len() {
        selected.insert(index);
    }

    if selected.is_empty() {
        let mut ranked = (0..records.len()).collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            records[*right]
                .score
                .cmp(&records[*left].score)
                .then_with(|| records[*left].line_number.cmp(&records[*right].line_number))
        });
        selected.extend(ranked.into_iter().take(12));
    }

    let items = records
        .iter()
        .map(|record| record.content.clone())
        .collect::<Vec<_>>();
    let cap = adaptive_keep_count(&items, 12, 60).max(8);
    if selected.len() > cap {
        let mut ranked = selected.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            records[*right]
                .score
                .cmp(&records[*left].score)
                .then_with(|| records[*left].line_number.cmp(&records[*right].line_number))
        });
        ranked.truncate(cap);
        selected = ranked.into_iter().collect();
    }

    selected
        .into_iter()
        .map(|index| records[index].clone())
        .collect()
}

#[derive(Clone)]
struct DiffFile {
    path: String,
    hunks: Vec<DiffHunk>,
    additions: usize,
    deletions: usize,
    order: usize,
}

#[derive(Clone)]
struct DiffHunk {
    header: String,
    lines: Vec<String>,
    additions: usize,
    deletions: usize,
    order: usize,
}

fn diff_signals(source_events: &[Event]) -> Vec<String> {
    let files = parse_diff_files(&joined_event_text(source_events));
    if files.is_empty() {
        return tool_output_signals(source_events);
    }

    let total_files = files.len();
    let total_hunks = files.iter().map(|file| file.hunks.len()).sum::<usize>();
    let additions = files.iter().map(|file| file.additions).sum::<usize>();
    let deletions = files.iter().map(|file| file.deletions).sum::<usize>();
    let file_items = files
        .iter()
        .map(|file| format!("{} {} {}", file.path, file.additions, file.deletions))
        .collect::<Vec<_>>();
    let keep_file_count = adaptive_keep_count(&file_items, 3, 8).max(1);

    let mut ranked_files = files;
    ranked_files.sort_by(|left, right| {
        let left_changes = left.additions + left.deletions;
        let right_changes = right.additions + right.deletions;
        right_changes
            .cmp(&left_changes)
            .then_with(|| left.order.cmp(&right.order))
    });

    let mut signals = Vec::new();
    let mut changed_files = Vec::new();
    let mut hunk_lines = Vec::new();
    let mut samples = Vec::new();
    let mut kept_hunks = 0usize;

    for file in ranked_files.iter().take(keep_file_count) {
        push_unique_owned(&mut changed_files, file.path.clone());
        let selected_hunks = select_diff_hunks(&file.hunks);
        kept_hunks += selected_hunks.len();
        for hunk in selected_hunks {
            hunk_lines.push(format!("{} {}", file.path, hunk.header));
            for line in representative_diff_lines(&hunk) {
                samples.push(format!("{} {}", file.path, line));
            }
        }
    }

    push_signal(
        &mut signals,
        format!(
            "Changed files: {}",
            changed_files
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    push_signal(
        &mut signals,
        format!(
            "Diff scale: files={}, kept_files={}, hunks={}, kept_hunks={}, omitted_hunks={}, additions={}, deletions={}",
            total_files,
            keep_file_count,
            total_hunks,
            kept_hunks,
            total_hunks.saturating_sub(kept_hunks),
            additions,
            deletions
        ),
    );
    append_labeled_items(&mut signals, "Diff hunk", hunk_lines);
    append_labeled_items(&mut signals, "Representative change", samples);
    signals
}

fn parse_diff_files(text: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    for line in text.lines().map(str::trim_end) {
        if line.starts_with("diff --git ") {
            flush_diff_hunk(&mut current_file, &mut current_hunk);
            flush_diff_file(&mut files, &mut current_file);
            current_file = Some(DiffFile {
                path: parse_diff_git_path(line).unwrap_or_else(|| "unknown".to_string()),
                hunks: Vec::new(),
                additions: 0,
                deletions: 0,
                order: files.len(),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("+++ b/") {
            if current_file.is_none() {
                current_file = Some(DiffFile {
                    path: path.trim().to_string(),
                    hunks: Vec::new(),
                    additions: 0,
                    deletions: 0,
                    order: files.len(),
                });
            } else if let Some(file) = current_file.as_mut() {
                file.path = path.trim().to_string();
            }
            continue;
        }

        if line.starts_with("@@ ") {
            if current_file.is_none() {
                current_file = Some(DiffFile {
                    path: "unknown".to_string(),
                    hunks: Vec::new(),
                    additions: 0,
                    deletions: 0,
                    order: files.len(),
                });
            }
            flush_diff_hunk(&mut current_file, &mut current_hunk);
            current_hunk = Some(DiffHunk {
                header: line.trim().to_string(),
                lines: Vec::new(),
                additions: 0,
                deletions: 0,
                order: current_file
                    .as_ref()
                    .map(|file| file.hunks.len())
                    .unwrap_or(0),
            });
            continue;
        }

        if let Some(hunk) = current_hunk.as_mut() {
            if line.starts_with('+') && !line.starts_with("+++") {
                hunk.additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                hunk.deletions += 1;
            }
            hunk.lines.push(line.to_string());
        }
    }

    flush_diff_hunk(&mut current_file, &mut current_hunk);
    flush_diff_file(&mut files, &mut current_file);
    files
}

fn flush_diff_hunk(current_file: &mut Option<DiffFile>, current_hunk: &mut Option<DiffHunk>) {
    let Some(hunk) = current_hunk.take() else {
        return;
    };
    let Some(file) = current_file.as_mut() else {
        return;
    };
    file.additions += hunk.additions;
    file.deletions += hunk.deletions;
    file.hunks.push(hunk);
}

fn flush_diff_file(files: &mut Vec<DiffFile>, current_file: &mut Option<DiffFile>) {
    let Some(file) = current_file.take() else {
        return;
    };
    if !file.hunks.is_empty() || file.additions > 0 || file.deletions > 0 {
        files.push(file);
    }
}

fn parse_diff_git_path(line: &str) -> Option<String> {
    line.split_whitespace()
        .nth(3)
        .and_then(|value| value.strip_prefix("b/").or(Some(value)))
        .map(str::to_string)
}

fn select_diff_hunks(hunks: &[DiffHunk]) -> Vec<DiffHunk> {
    if hunks.is_empty() {
        return Vec::new();
    }

    let items = hunks
        .iter()
        .map(|hunk| format!("{} {} {}", hunk.header, hunk.additions, hunk.deletions))
        .collect::<Vec<_>>();
    let keep_count = adaptive_keep_count(&items, 2, 6).max(1);
    let mut selected = BTreeSet::new();
    selected.insert(0usize);
    selected.insert(hunks.len() - 1);

    let mut ranked = (0..hunks.len()).collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        let left_changes = hunks[*left].additions + hunks[*left].deletions;
        let right_changes = hunks[*right].additions + hunks[*right].deletions;
        right_changes
            .cmp(&left_changes)
            .then_with(|| hunks[*left].order.cmp(&hunks[*right].order))
    });
    for index in ranked {
        if selected.len() >= keep_count {
            break;
        }
        selected.insert(index);
    }

    selected
        .into_iter()
        .map(|index| hunks[index].clone())
        .collect()
}

fn representative_diff_lines(hunk: &DiffHunk) -> Vec<String> {
    let mut samples = Vec::new();
    for line in &hunk.lines {
        if (line.starts_with('+') && !line.starts_with("+++"))
            || (line.starts_with('-') && !line.starts_with("---"))
        {
            push_unique_owned(&mut samples, truncate_preview(line.trim(), 140));
        }
        if samples.len() >= 4 {
            break;
        }
    }
    samples
}

fn provider_payload_signals(source_events: &[Event]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut kinds = Vec::new();
    let mut keys = Vec::new();
    let mut payload_bytes = 0usize;

    for block in source_events.iter().flat_map(|event| event.blocks.iter()) {
        let Block::Other { raw: payload } = block else {
            continue;
        };
        let kind = payload
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("other");
        push_unique(&mut kinds, kind);
        payload_bytes = payload_bytes.saturating_add(payload.to_string().len());
        if let Some(object) = payload.as_object() {
            for key in object.keys().take(8) {
                push_unique(&mut keys, key);
            }
        }
    }

    if !kinds.is_empty() {
        push_signal(&mut signals, format!("Payload kinds: {}", kinds.join(", ")));
    }
    if !keys.is_empty() {
        push_signal(&mut signals, format!("Top-level keys: {}", keys.join(", ")));
    }
    push_signal(&mut signals, format!("Payload bytes: {}", payload_bytes));
    signals
}

fn conversation_signals(source_events: &[Event]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut user_count = 0usize;
    let mut assistant_count = 0usize;
    let mut tool_count = 0usize;
    let mut first_user = None;
    let mut last_assistant = None;
    let mut paths = Vec::new();

    for event in source_events {
        match event.role {
            Role::User => {
                user_count += 1;
                if first_user.is_none() {
                    first_user = concise_event_text(event);
                }
            }
            Role::Assistant => {
                assistant_count += 1;
                last_assistant = concise_event_text(event);
            }
            Role::Tool => tool_count += 1,
            Role::System | Role::Developer | _ => {}
        }
        collect_path_mentions(&canonical_event_text(event), &mut paths);
    }

    push_signal(
        &mut signals,
        format!(
            "Roles: user={}, assistant={}, tool={}",
            user_count, assistant_count, tool_count
        ),
    );
    if let Some(first_user) = first_user {
        push_signal(&mut signals, format!("First user signal: {}", first_user));
    }
    if let Some(last_assistant) = last_assistant {
        push_signal(
            &mut signals,
            format!("Last assistant signal: {}", last_assistant),
        );
    }
    if !paths.is_empty() {
        push_signal(
            &mut signals,
            format!("Referenced paths: {}", paths.join(", ")),
        );
    }
    signals
}

fn collect_highlight_lines(text: &str, out: &mut Vec<String>) {
    let records = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| classify_log_line(index + 1, trimmed))
        })
        .collect::<Vec<_>>();
    for record in select_log_records(&records).into_iter().take(8) {
        push_unique_owned(out, truncate_preview(&record.content, 140));
    }
}

fn concise_event_text(event: &Event) -> Option<String> {
    let text = canonical_event_text(event);
    let text = text.trim();
    (!text.is_empty()).then(|| truncate_preview(text, 160))
}

fn collect_path_mentions(text: &str, out: &mut Vec<String>) {
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|ch: char| {
            matches!(
                ch,
                ',' | '.' | ':' | ';' | '"' | '\'' | '(' | ')' | '[' | ']'
            )
        });
        if cleaned.len() > 2
            && (cleaned.contains('/') || cleaned.contains('\\'))
            && cleaned
                .chars()
                .any(|ch| ch == '.' || ch == '/' || ch == '\\')
        {
            push_unique_owned(out, truncate_preview(cleaned, 120));
        }
        if out.len() >= 8 {
            return;
        }
    }
}

fn append_labeled_items(signals: &mut Vec<String>, label: &str, items: Vec<String>) {
    for item in items.into_iter().take(10) {
        push_signal(signals, format!("{}: {}", label, item));
    }
}

fn push_signal(signals: &mut Vec<String>, value: String) {
    if !value.trim().is_empty() && signals.len() < 16 {
        signals.push(value);
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    push_unique_owned(values, value.to_string());
}

fn push_unique_owned(values: &mut Vec<String>, value: String) {
    if !value.trim().is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn normalize_variable_tail(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            out.push('#');
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}
