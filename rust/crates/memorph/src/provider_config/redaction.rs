//! Defensive secret redaction for config views.
//!
//! Safety is primarily by construction — inspectors never place secret values into
//! rows (MCP environment is rendered as key names, HTTP headers as a count). This
//! pass is the backstop: any row whose label is explicitly a credential name
//! (`token`, `secret`, `password`, `auth`, `authorization`, …) is masked wholesale,
//! so a future inspector that accidentally drops a raw secret under such a label
//! still cannot leak it. The hints are deliberately the unambiguous credential
//! names only — `env`/`headers` are excluded because safe key/count displays live
//! under those labels and would be false-positive masked.
//!
//! Value-shape heuristics are avoided on purpose: they false-positive on URLs,
//! file paths and commit SHAs that the user needs to see. Model endpoints, API
//! keys and auth tokens are never surfaced as views at all.

use serde_json::Value;

use super::ConfigView;

const SECRET_LABEL_HINTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "auth",
    "bearer",
    "credential",
    "authorization",
];

pub fn is_secret_label(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    SECRET_LABEL_HINTS.iter().any(|hint| lower.contains(hint))
}

/// Mask every secret-labelled row in the view, recursively through nested values.
pub fn redact(view: &mut ConfigView) {
    for section in &mut view.sections {
        for row in &mut section.rows {
            if is_secret_label(&row.label) {
                row.value = mask(&row.value);
            }
        }
    }
}

fn mask(value: &Value) -> Value {
    match value {
        Value::String(_) => Value::String(MASK.to_string()),
        Value::Array(items) => Value::Array(items.iter().map(mask).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), mask(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

const MASK: &str = "••••••";

#[cfg(test)]
mod tests {
    use super::super::{ConfigRow, ConfigView};
    use super::redact;
    use serde_json::json;

    fn view_with(row: ConfigRow) -> ConfigView {
        let mut view = ConfigView::new("claude", "view_test", "test");
        view.push_section("section", vec![row]);
        view
    }

    #[test]
    fn masks_secret_labelled_rows() {
        let mut view = view_with(ConfigRow::fact("Authorization", "Bearer super-secret"));
        redact(&mut view);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("super-secret"));
        assert!(serialized.contains("••••••"));
    }

    #[test]
    fn masks_authorization_labelled_object() {
        let mut view = view_with(ConfigRow::fact(
            "Authorization",
            json!({ "Authorization": "Bearer hush", "X-Trace-Id": "abc" }),
        ));
        redact(&mut view);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("hush"));
        assert!(!serialized.contains("abc"));
    }

    #[test]
    fn leaves_urls_paths_and_shas_intact() {
        let mut view = ConfigView::new("claude", "view_test", "test");
        view.push_section(
            "section",
            vec![
                ConfigRow::fact("URL", "https://127.0.0.1:2923/mcp"),
                ConfigRow::fact("Commit", "b8ad3cc6c1e40b2d2a944f900a4ae0904a54dd7f"),
                ConfigRow::fact("Command", "/usr/local/bin/uvx"),
            ],
        );
        redact(&mut view);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(serialized.contains("https://127.0.0.1:2923/mcp"));
        assert!(serialized.contains("b8ad3cc6c1e40b2d2a944f900a4ae0904a54dd7f"));
        assert!(serialized.contains("/usr/local/bin/uvx"));
    }
}
