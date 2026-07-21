//! Minimal JSONC object helpers used by hook provider config installers.

use anyhow::{Context, Result};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy)]
struct JsonTopLevelProperty {
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

pub fn set_top_level_value(contents: &str, key: &str, value: &Value) -> Result<String> {
    if let Some(property) = find_jsonc_top_level_property(contents, key) {
        let mut output = String::new();
        output.push_str(&contents[..property.value_start]);
        output.push_str(&serde_json::to_string_pretty(value)?);
        output.push_str(&contents[property.value_end..]);
        return Ok(output);
    }

    let close_index = find_jsonc_top_level_object_close(contents)
        .with_context(|| "Failed to find top-level JSON object close")?;
    let parsed = parse_object(contents)?;
    let mut output = String::new();
    output.push_str(&contents[..close_index]);
    if parsed.is_empty() {
        output.push_str(&format!(
            "\n  \"{}\": {}\n",
            escape_json_key(key),
            serde_json::to_string_pretty(value)?
        ));
    } else {
        output.push_str(&format!(
            ",\n  \"{}\": {}\n",
            escape_json_key(key),
            serde_json::to_string_pretty(value)?
        ));
    }
    output.push_str(&contents[close_index..]);
    Ok(output)
}

pub fn delete_top_level_key(contents: &str, key: &str) -> Result<String> {
    let Some(property) = find_jsonc_top_level_property(contents, key) else {
        return Ok(contents.to_string());
    };
    let mut start = property.key_start;
    while start > 0 {
        let previous = contents.as_bytes()[start - 1];
        if previous == b' ' || previous == b'\t' {
            start -= 1;
            continue;
        }
        break;
    }
    let mut end = skip_jsonc_ws_comments(contents, property.value_end);
    if contents.as_bytes().get(end) == Some(&b',') {
        end += 1;
        if contents.as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
    } else {
        let mut previous = start;
        while previous > 0 && contents.as_bytes()[previous - 1].is_ascii_whitespace() {
            previous -= 1;
        }
        if previous > 0 && contents.as_bytes()[previous - 1] == b',' {
            start = previous - 1;
        }
    }
    let mut output = String::new();
    output.push_str(&contents[..start]);
    output.push_str(&contents[end..]);
    Ok(output)
}

fn find_jsonc_top_level_property(contents: &str, key: &str) -> Option<JsonTopLevelProperty> {
    let mut idx = 0;
    let mut depth = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        if ch == '"' && depth == 1 {
            let key_end = find_json_string_end(contents, idx)?;
            let parsed_key: String = serde_json::from_str(&contents[idx..key_end]).ok()?;
            let colon = skip_jsonc_ws_comments(contents, key_end);
            if contents.as_bytes().get(colon) == Some(&b':') {
                let value_start = skip_jsonc_ws_comments(contents, colon + 1);
                let value_end = find_jsonc_value_end(contents, value_start)?;
                if parsed_key == key {
                    return Some(JsonTopLevelProperty {
                        key_start: idx,
                        value_start,
                        value_end: trim_ascii_ws_end(contents, value_start, value_end),
                    });
                }
                idx = value_end;
                continue;
            }
        }
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' | '[' => {
                depth += 1;
                idx += ch.len_utf8();
            }
            '}' | ']' => {
                depth -= 1;
                idx += ch.len_utf8();
            }
            _ => idx += ch.len_utf8(),
        }
    }
    None
}

fn find_jsonc_top_level_object_close(contents: &str) -> Option<usize> {
    let mut idx = 0;
    let mut depth = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' => {
                depth += 1;
                idx += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
                idx += 1;
            }
            '[' => {
                depth += 1;
                idx += 1;
            }
            ']' => {
                depth -= 1;
                idx += 1;
            }
            _ => idx += ch.len_utf8(),
        }
    }
    None
}

fn find_jsonc_value_end(contents: &str, start: usize) -> Option<usize> {
    let mut idx = start;
    let mut nested = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' | '[' => {
                nested += 1;
                idx += 1;
            }
            '}' => {
                if nested == 0 {
                    return Some(idx);
                }
                nested -= 1;
                idx += 1;
            }
            ']' => {
                nested -= 1;
                idx += 1;
            }
            ',' if nested == 0 => return Some(idx),
            _ => idx += ch.len_utf8(),
        }
    }
    Some(contents.len())
}

fn find_json_string_end(contents: &str, start: usize) -> Option<usize> {
    let mut escaped = false;
    let mut iter = contents[start + 1..].char_indices();
    for (offset, ch) in iter {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(start + 1 + offset + ch.len_utf8());
        }
    }
    None
}

fn skip_jsonc_ws_comments(contents: &str, mut idx: usize) -> usize {
    loop {
        while idx < contents.len() && contents.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if contents[idx..].starts_with("//") {
            idx = contents[idx..]
                .find('\n')
                .map(|offset| idx + offset + 1)
                .unwrap_or(contents.len());
            continue;
        }
        if contents[idx..].starts_with("/*") {
            idx = contents[idx + 2..]
                .find("*/")
                .map(|offset| idx + 2 + offset + 2)
                .unwrap_or(contents.len());
            continue;
        }
        return idx;
    }
}

fn trim_ascii_ws_end(contents: &str, start: usize, mut end: usize) -> usize {
    while end > start && contents.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

pub fn ensure_trailing_newline(mut contents: String) -> String {
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents
}

fn escape_json_key(key: &str) -> String {
    serde_json::to_string(key)
        .unwrap_or_else(|_| format!("\"{key}\""))
        .trim_matches('"')
        .to_string()
}

pub fn parse_object(contents: &str) -> Result<Map<String, Value>> {
    match serde_json::from_str::<Value>(&strip_json_comments(contents))
        .context("Failed to parse JSON/JSONC object")?
    {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("Expected JSON object"),
    }
}

fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            result.push(ch);
            continue;
        }
        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for next in chars.by_ref() {
                        if prev == '*' && next == '/' {
                            break;
                        }
                        prev = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        result.push(ch);
    }
    result
}
