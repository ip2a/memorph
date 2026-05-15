use chrono::{DateTime, Utc};
use ratatui::style::Color;

/// 主题颜色配置
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    #[allow(dead_code)]
    pub surface: Color,
    pub text: Color,
    pub text_dim: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub border: Color,
    pub border_focused: Color,
    pub highlight: Color,
    pub provider_colors: [Color; 5],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: Color::Cyan,
            secondary: Color::Blue,
            accent: Color::Yellow,
            background: Color::Black,
            surface: Color::Rgb(30, 30, 30),
            text: Color::White,
            text_dim: Color::Gray,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            border: Color::Rgb(60, 60, 60),
            border_focused: Color::Cyan,
            highlight: Color::Rgb(40, 40, 40),
            provider_colors: [
                Color::Yellow,  // Claude
                Color::Green,   // Codex
                Color::Magenta, // OpenCode
                Color::Blue,    // Cursor
                Color::Cyan,    // Kiro
            ],
        }
    }
}

impl Theme {
    pub fn provider_color(&self, provider_id: &str) -> Color {
        match provider_id {
            "claude" => self.provider_colors[0],
            "codex" => self.provider_colors[1],
            "opencode" => self.provider_colors[2],
            "cursor" => self.provider_colors[3],
            "kiro" => self.provider_colors[4],
            _ => self.text_dim,
        }
    }
}

/// 格式化时间戳为相对时间
pub fn format_relative_time(timestamp: Option<i64>) -> String {
    let Some(ts) = timestamp else {
        return "-".to_string();
    };

    let now = Utc::now().timestamp();
    let diff = now - ts;

    if diff < 60 {
        "刚刚".to_string()
    } else if diff < 3600 {
        format!("{} 分钟前", diff / 60)
    } else if diff < 86400 {
        format!("{} 小时前", diff / 3600)
    } else if diff < 604800 {
        format!("{} 天前", diff / 86400)
    } else {
        let dt = DateTime::<Utc>::from_timestamp(ts, 0);
        dt.map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string())
    }
}

/// 截断字符串
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
