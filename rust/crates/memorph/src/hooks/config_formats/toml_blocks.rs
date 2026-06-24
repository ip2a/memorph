//! Helpers for TOML array-of-table hook blocks such as `[[hooks]]`.

pub fn blocks_from_contents(contents: &str, header: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim() == header {
            let mut block = vec![lines[idx].clone()];
            idx += 1;
            while idx < lines.len() {
                let trimmed = lines[idx].trim();
                if trimmed.starts_with('[') {
                    break;
                }
                block.push(lines[idx].clone());
                idx += 1;
            }
            blocks.push(block);
        } else {
            idx += 1;
        }
    }
    blocks
}

pub fn remove_blocks_matching<F>(contents: &str, header: &str, mut should_remove: F) -> String
where
    F: FnMut(&[String]) -> bool,
{
    let lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let had_trailing_newline = contents.ends_with('\n');
    let mut result = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim() == header {
            let mut block = vec![lines[idx].clone()];
            let mut next = idx + 1;
            while next < lines.len() {
                let trimmed = lines[next].trim();
                if trimmed.starts_with('[') {
                    break;
                }
                block.push(lines[next].clone());
                next += 1;
            }
            if !should_remove(&block) {
                result.extend(block);
            }
            idx = next;
        } else {
            result.push(lines[idx].clone());
            idx += 1;
        }
    }
    while result.last().is_some_and(|line| line.trim().is_empty()) {
        result.pop();
    }
    join_lines(result, had_trailing_newline)
}

pub fn block_string_assignment(block: &[String], key: &str) -> Option<String> {
    block
        .iter()
        .find_map(|line| toml_string_assignment_value(line.trim(), key))
}

pub fn block_contains_memorph_command(block: &[String]) -> bool {
    block
        .iter()
        .filter_map(|line| toml_string_assignment_value(line.trim(), "command"))
        .any(|command| crate::hooks::shared::command_contains_memorph_hook(&command))
}

pub fn block_memorph_command_version(block: &[String]) -> Option<String> {
    block
        .iter()
        .filter_map(|line| toml_string_assignment_value(line.trim(), "command"))
        .find_map(|command| {
            crate::hooks::shared::command_contains_memorph_hook(&command)
                .then(|| command_managed_version(&command))
        })
        .flatten()
}

fn command_managed_version(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|window| (window[0] == "--managed-version").then(|| window[1].to_string()))
}

fn toml_string_assignment_value(line: &str, key: &str) -> Option<String> {
    if toml_assignment_key(line) != Some(key) {
        return None;
    }
    let value = line.split_once('=')?.1.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        serde_json::from_str(value).ok()
    } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        Some(value[1..value.len() - 1].to_string())
    } else {
        Some(
            value
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string(),
        )
    }
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
