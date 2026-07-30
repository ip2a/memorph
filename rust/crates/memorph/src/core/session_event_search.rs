use crate::provider;
use crate::session::Event;

pub fn session_event_matches_query(event: &Event, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    session_event_search_haystack(event).contains(&query.to_ascii_lowercase())
}

pub fn find_matching_event_indices(events: &[Event], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..events.len()).collect();
    }
    events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| session_event_matches_query(event, query).then_some(index))
        .collect()
}

fn session_event_search_haystack(event: &Event) -> String {
    let mut parts = vec![
        event.id.clone(),
        serde_json::to_string(&event.role).unwrap_or_default(),
        serde_json::to_string(&event.kind).unwrap_or_default(),
    ];
    if let Some(model) = event.metadata.model.as_deref() {
        parts.push(model.to_string());
    }
    if let Ok(blocks_json) = serde_json::to_string(&event.blocks) {
        parts.push(blocks_json);
    }
    parts.push(provider::event_text(event));
    parts.join(" ").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Block, Event, EventKind, Metadata, Role};
    use chrono::TimeZone;

    fn sample_event(id: &str, text: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role: Role::User,
            timestamp: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            links: Default::default(),
            blocks: vec![Block::Text {
                text: text.to_string(),
            }],
            metadata: Metadata {
                model: None,
                usage: None,
            },
        }
    }

    #[test]
    fn matches_event_text_and_id() {
        let event = sample_event("evt-1", "hello world");
        assert!(session_event_matches_query(&event, "hello"));
        assert!(session_event_matches_query(&event, "evt-1"));
        assert!(!session_event_matches_query(&event, "missing"));
    }

    #[test]
    fn find_indices_preserves_order() {
        let events = vec![
            sample_event("a", "alpha"),
            sample_event("b", "beta"),
            sample_event("c", "alpha again"),
        ];
        assert_eq!(find_matching_event_indices(&events, "alpha"), vec![0, 2]);
    }

    #[test]
    fn empty_query_matches_all_indices() {
        let events = vec![sample_event("a", "one"), sample_event("b", "two")];
        assert_eq!(find_matching_event_indices(&events, ""), vec![0, 1]);
    }
}
