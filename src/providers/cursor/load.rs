use crate::model::{
    ContentBlock, MemorphMessage, MemorphMeta, MemorphRole, MemorphSession, SessionInfo,
};
use crate::providers::cursor::db::list_bubbles;
use anyhow::Result;
use chrono::{DateTime, Utc};

/// Load a full Cursor Composer session from bubble messages.
pub fn load_session(composer_id: &str) -> Result<MemorphSession> {
    let mut bubbles = list_bubbles(composer_id)?;

    // Sort by createdAt ascending
    bubbles.sort_by(|a, b| {
        let a_ts = a.created_at.as_deref().unwrap_or("");
        let b_ts = b.created_at.as_deref().unwrap_or("");
        a_ts.cmp(b_ts)
    });

    let mut messages = Vec::new();
    let mut turn_index = 0u32;

    for bubble in bubbles {
        let is_user = bubble.bubble_type == 1;
        let role = if is_user {
            MemorphRole::User
        } else if bubble.bubble_type == 2 {
            MemorphRole::Assistant
        } else {
            MemorphRole::System
        };

        let timestamp = bubble
            .created_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let content = bubble.text.unwrap_or_default().trim().to_string();

        // Skip empty non-assistant bubbles
        if content.is_empty() && !matches!(role, MemorphRole::Assistant) {
            continue;
        }

        // Increment turn index before pushing user message
        if is_user {
            turn_index += 1;
        }

        messages.push(MemorphMessage {
            id: bubble.bubble_id,
            role,
            content: vec![ContentBlock::Text { text: content }],
            timestamp,
            metadata: None,
            parent_id: None,
            turn_index: Some(turn_index),
        });
    }

    let first_active = messages.first().map(|m| m.timestamp);
    let last_active = messages.last().map(|m| m.timestamp);

    let title = messages
        .iter()
        .find(|m| matches!(m.role, MemorphRole::Assistant))
        .and_then(|m| m.content.first())
        .and_then(|block| match block {
            ContentBlock::Text { text } => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.chars().count() > 50 {
                    Some(format!(
                        "{}...",
                        trimmed.chars().take(50).collect::<String>()
                    ))
                } else {
                    Some(trimmed.to_string())
                }
            }
            _ => None,
        });

    Ok(MemorphSession {
        meta: MemorphMeta {
            version: "1.0".to_string(),
            converted_from: "cursor".to_string(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: composer_id.to_string(),
            source_provider: "cursor".to_string(),
            converted_by: Some("memorph-cli".to_string()),
        },
        session: SessionInfo {
            id: composer_id.to_string(),
            title,
            project_dir: None,
            created_at: first_active,
            last_active_at: last_active,
            tags: None,
        },
        messages,
    })
}
