use crate::config::UiLanguage;

pub(crate) fn parse_language(value: &str) -> Option<UiLanguage> {
    match value {
        "zh" | "zh-CN" => Some(UiLanguage::Zh),
        "en" | "en-US" => Some(UiLanguage::En),
        _ => None,
    }
}

pub(crate) fn lang_code(lang: UiLanguage) -> &'static str {
    match lang {
        UiLanguage::Zh => "zh",
        UiLanguage::En => "en",
    }
}

pub(crate) fn html_lang(lang: UiLanguage) -> &'static str {
    match lang {
        UiLanguage::Zh => "zh-CN",
        UiLanguage::En => "en",
    }
}

pub(crate) fn tr(lang: UiLanguage, zh: &'static str, en: &'static str) -> &'static str {
    match lang {
        UiLanguage::Zh => zh,
        UiLanguage::En => en,
    }
}

pub(crate) fn default_switch_target(from: &str) -> &'static str {
    match from {
        "codex" => "claude",
        _ => "codex",
    }
}

pub(crate) fn simple_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;

    for line in text.lines() {
        if line.starts_with("```") {
            if in_code_block {
                out.push_str("</code></pre>");
                in_code_block = false;
            } else {
                out.push_str("<pre><code>");
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            out.push_str(&html_escape(line));
            out.push('\n');
        } else if line.trim().is_empty() {
            out.push_str("<br>");
        } else {
            out.push_str(&inline_code(&html_escape(line)));
            out.push_str("<br>");
        }
    }

    if in_code_block {
        out.push_str("</code></pre>");
    }
    out
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn inline_code(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '`' {
            result.push_str("<code>");
            for inner in chars.by_ref() {
                if inner == '`' {
                    break;
                }
                result.push(inner);
            }
            result.push_str("</code>");
        } else {
            result.push(ch);
        }
    }

    result
}

pub(crate) fn truncate_json(value: &serde_json::Value, max_chars: usize) -> String {
    let raw = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    truncate_chars(&raw, max_chars)
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut result = value.chars().take(max_chars).collect::<String>();
    result.push_str("\n...");
    result
}

pub(crate) fn workspace_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .next_back()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub(crate) fn query_escape(value: &str) -> String {
    let mut escaped = String::new();

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                escaped.push(byte as char)
            }
            _ => escaped.push_str(&format!("%{byte:02X}")),
        }
    }

    escaped
}

pub(crate) fn home_url(
    workspace: &str,
    providers: &[String],
    search: Option<&str>,
    sort: Option<&str>,
    visible: usize,
    lang: UiLanguage,
) -> String {
    let mut params = vec![
        format!("workspace={}", query_escape(workspace)),
        format!("visible={visible}"),
        format!("lang={}", lang_code(lang)),
    ];

    for provider in providers.iter().filter(|value| !value.trim().is_empty()) {
        params.push(format!("provider={}", query_escape(provider)));
    }
    if let Some(search) = search.filter(|value| !value.trim().is_empty()) {
        params.push(format!("q={}", query_escape(search)));
    }
    if let Some(sort) = sort.filter(|value| !value.trim().is_empty()) {
        params.push(format!("sort={}", query_escape(sort)));
    }

    format!("/?{}", params.join("&"))
}
