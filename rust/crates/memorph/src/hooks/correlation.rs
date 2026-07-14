//! Correlate runtime hook sessions with memorph provider sessions.
//!
//! Hook runtime state is transient. Correlation links it back to provider-native
//! sessions so Web/Desktop/TUI can treat hooks as an enhancement layer instead
//! of a separate session system.

use anyhow::Result;
use rusqlite::Connection;

use crate::hooks::model::{HookEvent, RuntimeSessionCorrelation};
use crate::storage::snapshot_store::{ProjectedSessionIdentityRow, SnapshotStore};

pub fn correlate_event(event: &HookEvent) -> Option<RuntimeSessionCorrelation> {
    match try_correlate_event(event) {
        Ok(correlation) => correlation,
        Err(error) => {
            let _ = crate::hooks::store::append_error("correlate_event", error.to_string());
            None
        }
    }
}

fn try_correlate_event(event: &HookEvent) -> Result<Option<RuntimeSessionCorrelation>> {
    let Some(provider) = crate::providers::find_provider(&event.provider) else {
        return Ok(None);
    };
    let store =
        crate::storage::local_store::LocalSqliteStore::open(crate::hooks::store::database_path()?)?;
    correlate_event_from_projections(event, provider.as_ref(), store.connection())
}

fn correlate_event_from_projections(
    event: &HookEvent,
    provider: &dyn crate::provider::Provider,
    conn: &Connection,
) -> Result<Option<RuntimeSessionCorrelation>> {
    let snapshots = SnapshotStore::new(conn);
    if let Some(session_id) = event.provider_session_id.as_deref() {
        if let Some(identity) = snapshots.find_session_identity(&event.provider, session_id)? {
            return Ok(Some(correlation_from_identity(
                &event.provider,
                identity,
                "provider_session_id",
            )));
        }
    }

    let Some(cwd) = event
        .cwd
        .as_deref()
        .map(|path| path.to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let candidate = snapshots
        .list_provider_session_identities(&event.provider)?
        .into_iter()
        .find(|session| provider.workspace_matches(session.workspace_dir.as_deref(), Some(&cwd)));
    Ok(candidate.map(|identity| correlation_from_identity(&event.provider, identity, "workspace")))
}

fn correlation_from_identity(
    provider: &str,
    identity: ProjectedSessionIdentityRow,
    matched_by: &str,
) -> RuntimeSessionCorrelation {
    RuntimeSessionCorrelation {
        provider: provider.to_string(),
        session_id: identity
            .provider_session_id
            .unwrap_or(identity.canonical_session_id),
        title: identity.display_title.or(identity.title),
        project_dir: identity.workspace_dir,
        source_path: identity.source_path,
        matched_by: Some(matched_by.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{HookEvent, HookEventType};
    use crate::storage::local_store;
    use rusqlite::params;
    use serde_json::Value;
    use std::path::PathBuf;

    #[test]
    fn unknown_provider_has_no_correlation() {
        let mut event = HookEvent::new("unknown-provider", HookEventType::Heartbeat, Value::Null);
        event.provider_session_id = Some("s1".to_string());
        assert!(correlate_event(&event).is_none());
    }

    #[test]
    fn event_without_provider_session_or_cwd_has_no_correlation() {
        let event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let provider = crate::providers::find_provider("claude").unwrap();
        assert!(
            correlate_event_from_projections(&event, provider.as_ref(), &conn)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn event_with_missing_workspace_does_not_fail() {
        let mut event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        event.cwd = Some(PathBuf::from("/path/that/does/not/exist"));
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let provider = crate::providers::find_provider("claude").unwrap();
        assert!(
            correlate_event_from_projections(&event, provider.as_ref(), &conn)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn provider_session_id_correlation_survives_deleted_provider_source() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let source = tempfile::NamedTempFile::new().unwrap();
        let source_path = source.path().to_string_lossy().to_string();
        insert_projected_session(
            &conn,
            "canonical-1",
            "claude-session-1",
            "/tmp/project",
            &source_path,
            20,
        );
        drop(source);
        let mut event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        event.provider_session_id = Some("claude-session-1".to_string());
        let provider = crate::providers::find_provider("claude").unwrap();

        let correlation = correlate_event_from_projections(&event, provider.as_ref(), &conn)
            .unwrap()
            .expect("projected correlation");

        assert_eq!(correlation.session_id, "claude-session-1");
        assert_eq!(
            correlation.source_path.as_deref(),
            Some(source_path.as_str())
        );
        assert_eq!(
            correlation.matched_by.as_deref(),
            Some("provider_session_id")
        );
    }

    #[test]
    fn workspace_correlation_uses_latest_projected_session() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_session(
            &conn,
            "canonical-old",
            "claude-old",
            "/tmp/project",
            "/missing/old.jsonl",
            10,
        );
        insert_projected_session(
            &conn,
            "canonical-new",
            "claude-new",
            "/tmp/project",
            "/missing/new.jsonl",
            30,
        );
        let mut event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        event.cwd = Some(PathBuf::from("/tmp/project"));
        let provider = crate::providers::find_provider("claude").unwrap();

        let correlation = correlate_event_from_projections(&event, provider.as_ref(), &conn)
            .unwrap()
            .expect("workspace correlation");

        assert_eq!(correlation.session_id, "claude-new");
        assert_eq!(correlation.matched_by.as_deref(), Some("workspace"));
    }

    fn insert_projected_session(
        conn: &Connection,
        canonical_session_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        source_path: &str,
        last_active_at_ms: i64,
    ) {
        let source_id = format!("source-{canonical_session_id}");
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir,
              first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, 'claude', ?2, ?3, ?4, 10, ?5)",
            params![
                source_id,
                provider_session_id,
                source_path,
                workspace_dir,
                last_active_at_ms,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, 'claude', ?2, ?3, ?4, 'Projected title', 'completed', 10, ?5, 1, 1)",
            params![
                canonical_session_id,
                provider_session_id,
                source_id,
                workspace_dir,
                last_active_at_ms,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, workspace_dir, status, last_active_at_ms,
              event_count, turn_count, flags_json, projection_version, stale, updated_at_ms)
             VALUES (?1, 'claude', 'Projected title', ?2, 'completed', ?3, 1, 1,
                     '{}', 1, 0, ?3)",
            params![canonical_session_id, workspace_dir, last_active_at_ms],
        )
        .unwrap();
    }
}
