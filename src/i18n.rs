use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::config::UiLanguage;

#[derive(Debug, Deserialize)]
struct Catalog {
    zh: HashMap<String, String>,
    en: HashMap<String, String>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../web/i18n.json"))
            .expect("web/i18n.json must be valid JSON")
    })
}

fn lookup(language: UiLanguage, key: &str) -> Option<&'static str> {
    let map = match language {
        UiLanguage::Zh => &catalog().zh,
        UiLanguage::En => &catalog().en,
    };
    map.get(key).map(String::as_str)
}

pub fn text(language: UiLanguage, key: &'static str) -> &'static str {
    lookup(language, key)
        .or_else(|| catalog().zh.get(key).map(String::as_str))
        .unwrap_or(key)
}

pub fn format(language: UiLanguage, key: &'static str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = text(language, key).to_string();
    for (name, value) in replacements {
        rendered = rendered.replace(&format!("{{{name}}}"), value);
    }
    rendered
}

pub fn document_lang(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::Zh => "zh-CN",
        UiLanguage::En => "en",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_translations_from_shared_catalog() {
        assert_eq!(text(UiLanguage::Zh, "settings"), "设置");
        assert_eq!(text(UiLanguage::En, "settings"), "Settings");
    }

    #[test]
    fn falls_back_to_zh_for_missing_language_entry() {
        assert_eq!(text(UiLanguage::En, "languageNativeZh"), "中文");
    }

    #[test]
    fn formats_named_placeholders() {
        assert_eq!(
            format(UiLanguage::En, "sessionGroupCount", &[("count", "3")]),
            "3 sessions"
        );
        assert_eq!(
            format(
                UiLanguage::Zh,
                "showingRange",
                &[("start", "1"), ("end", "5"), ("total", "9")]
            ),
            "显示 1-5 / 9"
        );
    }
}
