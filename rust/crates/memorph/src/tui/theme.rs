use chrono::{DateTime, Utc};
use ratatui::style::Color;
use std::collections::HashMap;

use crate::config::UiLanguage;
use crate::i18n;
use crate::providers;

/// Theme color configuration
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    #[allow(dead_code)]
    pub surface: Color,
    pub surface_active: Color,
    pub text: Color,
    pub text_dim: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub border: Color,
    pub border_focused: Color,
    pub highlight: Color,
    pub provider_colors: HashMap<&'static str, Color>,
}

impl Default for Theme {
    fn default() -> Self {
        let palette = [
            Color::Rgb(150, 80, 0),
            Color::Green,
            Color::Magenta,
            Color::Blue,
            Color::Cyan,
            Color::Rgb(0, 128, 255),
            Color::Rgb(255, 165, 0),
            Color::Rgb(128, 0, 128),
            Color::Rgb(0, 150, 136),
            Color::Rgb(233, 30, 99),
        ];
        let mut provider_colors = HashMap::new();
        for (i, id) in providers::all_provider_ids().iter().enumerate() {
            provider_colors.insert(*id, palette[i % palette.len()]);
        }

        Self {
            primary: Color::Blue,
            secondary: Color::Blue,
            accent: Color::Magenta,
            background: Color::White,
            surface: Color::White,
            surface_active: Color::Rgb(244, 248, 255),
            text: Color::Black,
            text_dim: Color::Black,
            success: Color::Green,
            warning: Color::Rgb(184, 92, 0),
            error: Color::Red,
            border: Color::Black,
            border_focused: Color::Blue,
            highlight: Color::Rgb(220, 235, 255),
            provider_colors,
        }
    }
}

impl Theme {
    pub fn provider_color(&self, provider_id: &str) -> Color {
        self.provider_colors
            .get(provider_id)
            .copied()
            .unwrap_or(self.text_dim)
    }
}

/// Format timestamp as relative time
pub fn format_relative_time(timestamp: Option<i64>, language: UiLanguage) -> String {
    let Some(ts) = timestamp else {
        return "-".to_string();
    };

    let ts = normalize_timestamp_secs(ts);
    let now = Utc::now().timestamp();
    let diff = now - ts;

    if diff < 60 {
        i18n::text(language, "justNow").to_string()
    } else if diff < 3600 {
        i18n::format(
            language,
            "minutesAgo",
            &[("count", &(diff / 60).to_string())],
        )
    } else if diff < 86400 {
        i18n::format(
            language,
            "hoursAgo",
            &[("count", &(diff / 3600).to_string())],
        )
    } else if diff < 604800 {
        i18n::format(
            language,
            "daysAgo",
            &[("count", &(diff / 86400).to_string())],
        )
    } else {
        let dt = DateTime::<Utc>::from_timestamp(ts, 0);
        dt.map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string())
    }
}

fn normalize_timestamp_secs(timestamp: i64) -> i64 {
    if timestamp.abs() >= 100_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    }
}

/// Truncate string
pub fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max_chars - 3).collect();
        result.push_str("...");
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn formats_millisecond_timestamps_as_elapsed_time() {
        let two_hours_ago = (Utc::now() - Duration::hours(2)).timestamp_millis();

        assert_eq!(
            format_relative_time(Some(two_hours_ago), UiLanguage::En),
            "2 hours ago"
        );
    }

    #[test]
    fn keeps_second_timestamps_supported() {
        let three_days_ago = (Utc::now() - Duration::days(3)).timestamp();

        assert_eq!(
            format_relative_time(Some(three_days_ago), UiLanguage::Zh),
            "3 天前"
        );
    }
}
