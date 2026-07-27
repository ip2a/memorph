//! Hook event and runtime session persistence.
//!
//! Provider-native session files remain provider-owned. Hook events, runtime
//! observations, errors, and the local hook endpoint are memorph management
//! data and are stored in the local SQLite database.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{OnceLock, RwLock};

use crate::hooks::model::{HookEvent, RuntimeSession};
use crate::hooks::protocol::HookRuntimeEndpoint;
use crate::storage::local_store::LocalSqliteStore;

const HOOK_RUNTIME_ENDPOINT_ID: &str = "hook-server";
const HOOK_RUNTIME_KIND: &str = "hook_server";
const RAW_HOOK_EVENT_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const RAW_HOOK_EVENT_MAX_ROWS: i64 = 50_000;

#[cfg(any(test, feature = "test-support"))]
static TEST_STORE_ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct HookEventRetentionPolicy {
    max_age_ms: i64,
    max_rows: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HookEventRetentionReport {
    expired_rows_deleted: usize,
    excess_rows_deleted: usize,
}

const RAW_HOOK_EVENT_RETENTION: HookEventRetentionPolicy = HookEventRetentionPolicy {
    max_age_ms: RAW_HOOK_EVENT_RETENTION_MS,
    max_rows: RAW_HOOK_EVENT_MAX_ROWS,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSessionStore {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub sessions: Vec<RuntimeSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookErrorRecord {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub scope: String,
    pub message: String,
}

fn current_version() -> u32 {
    1
}

impl Default for RuntimeSessionStore {
    fn default() -> Self {
        Self {
            version: current_version(),
            sessions: Vec::new(),
        }
    }
}

pub fn database_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(root) = test_store_root() {
        return Ok(root.join("memorph.db"));
    }

    crate::storage::local_store::database_path()
}

fn open_store() -> Result<LocalSqliteStore> {
    LocalSqliteStore::open(database_path()?)
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_test_store_root(root: PathBuf) {
    let lock = TEST_STORE_ROOT.get_or_init(|| RwLock::new(None));
    *lock.write().unwrap() = Some(root);
}

#[cfg(test)]
fn test_store_root() -> Option<PathBuf> {
    TEST_STORE_ROOT
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap()
        .clone()
}

pub fn append_event(event: &HookEvent) -> Result<()> {
    let mut store = open_store()?;
    append_event_in(store.connection_mut(), event)?;
    Ok(())
}

fn append_event_in(conn: &mut Connection, event: &HookEvent) -> Result<HookEventRetentionReport> {
    append_event_with_retention_in(
        conn,
        event,
        chrono::Utc::now().timestamp_millis(),
        RAW_HOOK_EVENT_RETENTION,
    )
}

fn append_event_with_retention_in(
    conn: &mut Connection,
    event: &HookEvent,
    now_ms: i64,
    policy: HookEventRetentionPolicy,
) -> Result<HookEventRetentionReport> {
    let payload_json = serde_json::to_string(event).context("Failed to encode hook event")?;
    let event_name =
        serde_json::to_string(&event.event_type).context("Failed to encode hook event type")?;
    let event_name = event_name.trim_matches('"');
    let cutoff_ms = now_ms.saturating_sub(policy.max_age_ms);
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start hook event append transaction")?;
    let expired_rows_deleted = tx
        .execute(
            "DELETE FROM hook_events WHERE observed_at_ms < ?1",
            [cutoff_ms],
        )
        .context("Failed to prune expired hook events")?;
    let session_id =
        resolve_session_id(&tx, &event.provider, event.provider_session_id.as_deref())?;
    tx.execute(
        "INSERT INTO hook_events
         (id, provider_id, provider_session_id, session_id, event_name, observed_at_ms,
          correlation_id, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event.event_id,
            event.provider,
            event.provider_session_id,
            session_id,
            event_name,
            event.timestamp.timestamp_millis(),
            event.run_id,
            payload_json,
        ],
    )
    .context("Failed to insert hook event")?;
    let excess_rows_deleted = tx
        .execute(
            "DELETE FROM hook_events
             WHERE rowid IN (
                 SELECT rowid
                 FROM hook_events
                 ORDER BY observed_at_ms DESC, rowid DESC
                 LIMIT -1 OFFSET ?1
             )",
            [policy.max_rows],
        )
        .context("Failed to enforce hook event row limit")?;
    tx.commit()
        .context("Failed to commit hook event append transaction")?;
    Ok(HookEventRetentionReport {
        expired_rows_deleted,
        excess_rows_deleted,
    })
}

pub fn load_recent_events(limit: usize) -> Result<Vec<HookEvent>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let store = open_store()?;
    load_recent_events_in(store.connection(), limit)
}

pub fn last_event_observed_at_ms_for_providers(
    provider_ids: &[String],
) -> Result<std::collections::HashMap<String, i64>> {
    if provider_ids.is_empty() {
        return Ok(Default::default());
    }
    let store = open_store()?;
    last_event_observed_at_ms_for_providers_in(store.connection(), provider_ids)
}

pub fn last_event_observed_at_ms_for_providers_in(
    conn: &Connection,
    provider_ids: &[String],
) -> Result<std::collections::HashMap<String, i64>> {
    if provider_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = vec!["?"; provider_ids.len()].join(", ");
    let sql = format!(
        "SELECT provider_id, MAX(observed_at_ms)
         FROM hook_events
         WHERE provider_id IN ({placeholders})
         GROUP BY provider_id"
    );
    let params: Vec<&dyn rusqlite::ToSql> = provider_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare hook event provider max query")?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            let provider_id: String = row.get(0)?;
            let max_ms: i64 = row.get(1)?;
            Ok((provider_id, max_ms))
        })
        .context("Failed to query hook event provider max")?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (provider_id, max_ms) = row?;
        out.insert(provider_id, max_ms);
    }
    Ok(out)
}

fn load_recent_events_in(conn: &Connection, limit: usize) -> Result<Vec<HookEvent>> {
    let mut stmt = conn
        .prepare(
            "SELECT payload_json
             FROM hook_events
             ORDER BY observed_at_ms DESC, rowid DESC
             LIMIT ?1",
        )
        .context("Failed to prepare recent hook events query")?;
    let rows = stmt
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .context("Failed to query recent hook events")?;
    let mut events = Vec::new();
    for row in rows {
        let payload = row.context("Failed to decode hook event row")?;
        events.push(
            serde_json::from_str(&payload).context("Failed to decode stored hook event payload")?,
        );
    }
    events.reverse();
    Ok(events)
}

pub fn append_error(scope: impl Into<String>, message: impl Into<String>) -> Result<()> {
    let store = open_store()?;
    append_error_in(store.connection(), scope.into(), message.into())
}

fn append_error_in(conn: &Connection, scope: String, message: String) -> Result<()> {
    let record = HookErrorRecord {
        timestamp: chrono::Utc::now(),
        scope,
        message,
    };
    let details_json =
        serde_json::to_string(&record).context("Failed to encode hook error record")?;
    conn.execute(
        "INSERT INTO hook_errors
         (id, scope, message, observed_at_ms, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            record.scope,
            record.message,
            record.timestamp.timestamp_millis(),
            details_json,
        ],
    )
    .context("Failed to insert hook error")?;
    Ok(())
}

pub fn load_recent_errors(limit: usize) -> Result<Vec<HookErrorRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let store = open_store()?;
    load_recent_errors_in(store.connection(), limit)
}

fn load_recent_errors_in(conn: &Connection, limit: usize) -> Result<Vec<HookErrorRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT details_json
             FROM hook_errors
             ORDER BY observed_at_ms DESC, rowid DESC
             LIMIT ?1",
        )
        .context("Failed to prepare recent hook errors query")?;
    let rows = stmt
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .context("Failed to query recent hook errors")?;
    let mut errors = Vec::new();
    for row in rows {
        let details = row.context("Failed to decode hook error row")?;
        errors.push(
            serde_json::from_str(&details).context("Failed to decode stored hook error record")?,
        );
    }
    errors.reverse();
    Ok(errors)
}

pub fn save_runtime_sessions(runtime_store: &RuntimeSessionStore) -> Result<()> {
    let mut store = open_store()?;
    save_runtime_sessions_in(store.connection_mut(), runtime_store)
}

fn save_runtime_sessions_in(
    conn: &mut Connection,
    runtime_store: &RuntimeSessionStore,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start runtime session snapshot transaction")?;
    let current_ids = runtime_store
        .sessions
        .iter()
        .map(|session| session.runtime_id.0.as_str())
        .collect::<std::collections::HashSet<_>>();

    for session in &runtime_store.sessions {
        let details_json =
            serde_json::to_string(session).context("Failed to encode runtime session")?;
        let status =
            serde_json::to_string(&session.status).context("Failed to encode runtime status")?;
        let status = status.trim_matches('"');
        let correlation_provider = session
            .correlation
            .as_ref()
            .map(|correlation| correlation.provider.as_str())
            .unwrap_or(session.provider.as_str());
        let provider_session_id = session.provider_session_id.as_deref().or_else(|| {
            session
                .correlation
                .as_ref()
                .map(|correlation| correlation.session_id.as_str())
        });
        let session_id = resolve_session_id(&tx, correlation_provider, provider_session_id)?;
        let workspace_dir = session
            .cwd
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned());
        tx.execute(
            "INSERT INTO runtime_session_observations
             (id, provider_id, provider_session_id, session_id, workspace_dir, status,
              correlation_id, observed_at_ms, recent_activity_at_ms, details_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
              provider_id = excluded.provider_id,
              provider_session_id = excluded.provider_session_id,
              session_id = excluded.session_id,
              workspace_dir = excluded.workspace_dir,
              status = excluded.status,
              correlation_id = excluded.correlation_id,
              observed_at_ms = excluded.observed_at_ms,
              recent_activity_at_ms = excluded.recent_activity_at_ms,
              details_json = excluded.details_json",
            params![
                session.runtime_id.0,
                session.provider,
                provider_session_id,
                session_id,
                workspace_dir,
                status,
                session.run_id,
                session.updated_at.timestamp_millis(),
                session.last_event_at.timestamp_millis(),
                details_json,
            ],
        )
        .context("Failed to upsert runtime session observation")?;
    }

    let existing_ids = {
        let mut stmt = tx
            .prepare("SELECT id FROM runtime_session_observations")
            .context("Failed to prepare runtime session observation lookup")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("Failed to query runtime session observation ids")?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.context("Failed to decode runtime session observation id")?);
        }
        ids
    };
    for existing_id in existing_ids {
        if current_ids.contains(existing_id.as_str()) {
            continue;
        }
        tx.execute(
            "DELETE FROM runtime_session_observations WHERE id = ?1",
            [existing_id],
        )
        .context("Failed to delete removed runtime session observation")?;
    }

    tx.commit()
        .context("Failed to commit runtime session snapshot")
}

pub fn load_runtime_sessions() -> Result<RuntimeSessionStore> {
    let store = open_store()?;
    load_runtime_sessions_in(store.connection())
}

fn load_runtime_sessions_in(conn: &Connection) -> Result<RuntimeSessionStore> {
    let mut stmt = conn
        .prepare(
            "SELECT details_json
             FROM runtime_session_observations
             ORDER BY observed_at_ms, id",
        )
        .context("Failed to prepare runtime session observations query")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .context("Failed to query runtime session observations")?;
    let mut sessions = Vec::new();
    for row in rows {
        let details = row.context("Failed to decode runtime session observation row")?;
        sessions.push(
            serde_json::from_str(&details).context("Failed to decode stored runtime session")?,
        );
    }
    Ok(RuntimeSessionStore {
        version: current_version(),
        sessions,
    })
}

pub fn save_server_runtime(endpoint: &HookRuntimeEndpoint) -> Result<()> {
    let store = open_store()?;
    save_server_runtime_in(store.connection(), endpoint)
}

fn save_server_runtime_in(conn: &Connection, endpoint: &HookRuntimeEndpoint) -> Result<()> {
    let metadata_json =
        serde_json::to_string(endpoint).context("Failed to encode hook runtime endpoint")?;
    let parsed = reqwest::Url::parse(&endpoint.endpoint).ok();
    let host = parsed
        .as_ref()
        .and_then(|url| url.host_str())
        .map(str::to_string);
    let port = parsed
        .as_ref()
        .and_then(|url| url.port_or_known_default())
        .map(i64::from);
    let published_at_ms = endpoint.started_at.timestamp_millis();
    let token_hash = format!("{:x}", md5::compute(endpoint.token.as_bytes()));
    conn.execute(
        "INSERT INTO runtime_endpoints
         (id, runtime_kind, pid, host, port, base_url, token_hash, published_at_ms,
          last_seen_at_ms, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
          runtime_kind = excluded.runtime_kind,
          pid = excluded.pid,
          host = excluded.host,
          port = excluded.port,
          base_url = excluded.base_url,
          token_hash = excluded.token_hash,
          published_at_ms = excluded.published_at_ms,
          expires_at_ms = NULL,
          last_seen_at_ms = excluded.last_seen_at_ms,
          metadata_json = excluded.metadata_json",
        params![
            HOOK_RUNTIME_ENDPOINT_ID,
            HOOK_RUNTIME_KIND,
            i64::from(endpoint.pid),
            host,
            port,
            endpoint.endpoint,
            token_hash,
            published_at_ms,
            metadata_json,
        ],
    )
    .context("Failed to upsert hook runtime endpoint")?;
    Ok(())
}

pub fn load_server_runtime() -> Result<Option<HookRuntimeEndpoint>> {
    let store = open_store()?;
    load_server_runtime_in(store.connection())
}

fn load_server_runtime_in(conn: &Connection) -> Result<Option<HookRuntimeEndpoint>> {
    let metadata_json = conn
        .query_row(
            "SELECT metadata_json
             FROM runtime_endpoints
             WHERE id = ?1 AND runtime_kind = ?2
             LIMIT 1",
            params![HOOK_RUNTIME_ENDPOINT_ID, HOOK_RUNTIME_KIND],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("Failed to query hook runtime endpoint")?;
    metadata_json
        .map(|metadata| {
            serde_json::from_str(&metadata).context("Failed to decode stored hook runtime endpoint")
        })
        .transpose()
}

fn resolve_session_id(
    conn: &Connection,
    provider_id: &str,
    provider_session_id: Option<&str>,
) -> Result<Option<String>> {
    let Some(provider_session_id) = provider_session_id else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT s.id
         FROM sessions s
         WHERE s.deleted_at_ms IS NULL
           AND s.provider_id = ?1
           AND (
                s.provider_session_id = ?2
                OR s.id = ?2
                OR EXISTS (
                    SELECT 1
                    FROM session_aliases alias
                    WHERE alias.session_id = s.id
                      AND alias.provider_id = ?1
                      AND alias.alias_kind = 'provider_session_id'
                      AND alias.alias_value = ?2
                )
           )
         ORDER BY COALESCE(s.last_active_at_ms, s.updated_at_ms, s.created_at_ms) DESC
         LIMIT 1",
        params![provider_id, provider_session_id],
        |row| row.get(0),
    )
    .optional()
    .context("Failed to resolve canonical session for hook observation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{
        HookEventType, RuntimeSessionCorrelation, RuntimeSessionId, RuntimeSessionStatus,
    };
    use crate::storage::local_store;
    use serde_json::{json, Value};

    fn test_store() -> LocalSqliteStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep().join("memorph.db");
        LocalSqliteStore::open(path).unwrap()
    }

    fn runtime_session(id: &str, updated_at: chrono::DateTime<chrono::Utc>) -> RuntimeSession {
        RuntimeSession {
            runtime_id: RuntimeSessionId::new(id),
            provider: "generic".to_string(),
            provider_session_id: Some(format!("provider-{id}")),
            run_id: Some(format!("run-{id}")),
            cwd: Some(PathBuf::from("/tmp/project")),
            pid: Some(42),
            parent_pid: Some(1),
            pid_start_time: Some("100".to_string()),
            tty: Some("ttys001".to_string()),
            terminal_vars: [("TERM".to_string(), "xterm".to_string())]
                .into_iter()
                .collect(),
            process_ancestry: Vec::new(),
            correlation: Some(RuntimeSessionCorrelation {
                provider: "generic".to_string(),
                session_id: format!("provider-{id}"),
                title: Some("Session".to_string()),
                project_dir: Some("/tmp/project".to_string()),
                source_path: Some("/tmp/session.jsonl".to_string()),
                matched_by: Some("provider_session_id".to_string()),
            }),
            model: Some("model".to_string()),
            session_title: Some("Session".to_string()),
            transcript_path: Some(PathBuf::from("/tmp/session.jsonl")),
            workspace_roots: vec![PathBuf::from("/tmp/project")],
            last_user_prompt: Some("prompt".to_string()),
            last_assistant_message: Some("response".to_string()),
            last_tool_result: Some("result".to_string()),
            last_error: None,
            stop_reason: None,
            compact_count: 1,
            tool_call_count: 2,
            failed_tool_count: 0,
            permission_request_count: 1,
            question_count: 1,
            status: RuntimeSessionStatus::Running,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: Default::default(),
            last_event_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn appends_and_loads_recent_events_in_stable_order() {
        let mut store = test_store();
        let timestamp = chrono::Utc::now();
        for idx in 0..3 {
            let mut event = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
            event.event_id = format!("event-{idx}");
            event.timestamp = timestamp;
            append_event_in(store.connection_mut(), &event).unwrap();
        }

        let events = load_recent_events_in(store.connection(), 2).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "event-1");
        assert_eq!(events[1].event_id, "event-2");
    }

    #[test]
    fn last_event_observed_at_ms_for_providers_returns_max_per_provider() {
        let mut store = test_store();
        let base = chrono::Utc::now();

        let mut early = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        early.event_id = "claude-early".to_string();
        early.timestamp = base - chrono::Duration::seconds(100);
        append_event_in(store.connection_mut(), &early).unwrap();

        let mut late = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        late.event_id = "claude-late".to_string();
        late.timestamp = base;
        append_event_in(store.connection_mut(), &late).unwrap();

        let mut codex_event = HookEvent::new("codex", HookEventType::Heartbeat, Value::Null);
        codex_event.event_id = "codex-1".to_string();
        codex_event.timestamp = base - chrono::Duration::seconds(10);
        append_event_in(store.connection_mut(), &codex_event).unwrap();

        let map = last_event_observed_at_ms_for_providers_in(
            store.connection(),
            &["claude".to_string(), "codex".to_string()],
        )
        .unwrap();

        assert_eq!(map.len(), 2);
        assert_eq!(map["claude"], base.timestamp_millis());
        assert_eq!(map["codex"], codex_event.timestamp.timestamp_millis());

        let empty = last_event_observed_at_ms_for_providers_in(store.connection(), &[]).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn event_retention_prunes_expired_rows_only() {
        let mut store = test_store();
        let now = chrono::Utc::now();
        let policy = HookEventRetentionPolicy {
            max_age_ms: 1_000,
            max_rows: 10,
        };
        let mut expired = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        expired.event_id = "expired".to_string();
        expired.timestamp = now - chrono::Duration::seconds(2);
        append_event_with_retention_in(
            store.connection_mut(),
            &expired,
            expired.timestamp.timestamp_millis(),
            policy,
        )
        .unwrap();
        append_error_in(
            store.connection(),
            "test".to_string(),
            "keep error".to_string(),
        )
        .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO runtime_session_observations
                 (id, provider_id, status, observed_at_ms, details_json)
                 VALUES ('runtime-keep', 'generic', 'running', ?1, '{}')",
                [expired.timestamp.timestamp_millis()],
            )
            .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO session_activity
                 (id, operation_kind, status, started_at_ms)
                 VALUES ('activity-keep', 'scan', 'success', ?1)",
                [expired.timestamp.timestamp_millis()],
            )
            .unwrap();

        let mut current = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        current.event_id = "current".to_string();
        current.timestamp = now;
        let report = append_event_with_retention_in(
            store.connection_mut(),
            &current,
            now.timestamp_millis(),
            policy,
        )
        .unwrap();

        assert_eq!(
            report,
            HookEventRetentionReport {
                expired_rows_deleted: 1,
                excess_rows_deleted: 0,
            }
        );
        assert_eq!(
            load_recent_events_in(store.connection(), 10)
                .unwrap()
                .into_iter()
                .map(|event| event.event_id)
                .collect::<Vec<_>>(),
            vec!["current"]
        );
        for table in [
            "hook_errors",
            "runtime_session_observations",
            "session_activity",
        ] {
            let count: i64 = store
                .connection()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "{table} should not be pruned");
        }
    }

    #[test]
    fn event_retention_row_cap_keeps_newest_event_timestamps() {
        let mut store = test_store();
        let base = chrono::Utc::now();
        let policy = HookEventRetentionPolicy {
            max_age_ms: 60_000,
            max_rows: 3,
        };
        for seconds in [10, 30, 20, 40] {
            let mut event = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
            event.event_id = format!("event-{seconds}");
            event.timestamp = base + chrono::Duration::seconds(seconds);
            append_event_with_retention_in(
                store.connection_mut(),
                &event,
                base.timestamp_millis(),
                policy,
            )
            .unwrap();
        }

        assert_eq!(
            load_recent_events_in(store.connection(), 10)
                .unwrap()
                .into_iter()
                .map(|event| event.event_id)
                .collect::<Vec<_>>(),
            vec!["event-20", "event-30", "event-40"]
        );
    }

    #[test]
    fn event_retention_row_cap_uses_stable_row_order_for_equal_timestamps() {
        let mut store = test_store();
        let timestamp = chrono::Utc::now();
        let policy = HookEventRetentionPolicy {
            max_age_ms: 60_000,
            max_rows: 3,
        };
        let mut last_report = HookEventRetentionReport::default();
        for idx in 0..4 {
            let mut event = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
            event.event_id = format!("event-{idx}");
            event.timestamp = timestamp;
            last_report = append_event_with_retention_in(
                store.connection_mut(),
                &event,
                timestamp.timestamp_millis(),
                policy,
            )
            .unwrap();
        }

        assert_eq!(last_report.excess_rows_deleted, 1);
        assert_eq!(
            load_recent_events_in(store.connection(), 10)
                .unwrap()
                .into_iter()
                .map(|event| event.event_id)
                .collect::<Vec<_>>(),
            vec!["event-1", "event-2", "event-3"]
        );
    }

    #[test]
    fn failed_event_insert_rolls_back_retention() {
        let mut store = test_store();
        let now = chrono::Utc::now();
        let policy = HookEventRetentionPolicy {
            max_age_ms: 1_000,
            max_rows: 10,
        };
        let mut duplicate = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        duplicate.event_id = "duplicate".to_string();
        duplicate.timestamp = now;
        append_event_with_retention_in(
            store.connection_mut(),
            &duplicate,
            now.timestamp_millis(),
            policy,
        )
        .unwrap();
        let mut expired = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        expired.event_id = "expired".to_string();
        expired.timestamp = now - chrono::Duration::seconds(2);
        append_event_with_retention_in(
            store.connection_mut(),
            &expired,
            expired.timestamp.timestamp_millis(),
            policy,
        )
        .unwrap();

        let error = append_event_with_retention_in(
            store.connection_mut(),
            &duplicate,
            now.timestamp_millis(),
            policy,
        )
        .unwrap_err();

        assert!(error.to_string().contains("Failed to insert hook event"));
        let expired_exists: bool = store
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM hook_events WHERE id = 'expired')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(expired_exists);
    }

    #[test]
    fn appends_and_loads_recent_errors_in_stable_order() {
        let store = test_store();
        for idx in 0..3 {
            append_error_in(
                store.connection(),
                "test".to_string(),
                format!("error-{idx}"),
            )
            .unwrap();
        }

        let errors = load_recent_errors_in(store.connection(), 2).unwrap();

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].message, "error-1");
        assert_eq!(errors[1].message, "error-2");
    }

    #[test]
    fn runtime_snapshot_roundtrips_and_removes_absent_sessions() {
        let mut store = test_store();
        let now = chrono::Utc::now();
        let first = runtime_session("runtime-1", now);
        let second = runtime_session("runtime-2", now + chrono::Duration::seconds(1));
        save_runtime_sessions_in(
            store.connection_mut(),
            &RuntimeSessionStore {
                version: 1,
                sessions: vec![first.clone(), second],
            },
        )
        .unwrap();

        let loaded = load_runtime_sessions_in(store.connection()).unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.sessions[0], first);

        save_runtime_sessions_in(
            store.connection_mut(),
            &RuntimeSessionStore {
                version: 1,
                sessions: vec![first.clone()],
            },
        )
        .unwrap();
        let loaded = load_runtime_sessions_in(store.connection()).unwrap();
        assert_eq!(loaded.sessions, vec![first]);
    }

    #[test]
    fn runtime_snapshot_waits_for_concurrent_writer_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memorph.db");
        let mut blocker = LocalSqliteStore::open(&path).unwrap();
        let mut writer = LocalSqliteStore::open(&path).unwrap();
        let blocker_tx = blocker
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        blocker_tx
            .execute(
                "INSERT INTO hook_errors
                 (id, scope, message, observed_at_ms, details_json)
                 VALUES ('blocker', 'test', 'blocking write', 0, '{}')",
                [],
            )
            .unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writer_barrier = barrier.clone();
        let handle = std::thread::spawn(move || {
            writer_barrier.wait();
            save_runtime_sessions_in(
                writer.connection_mut(),
                &RuntimeSessionStore {
                    version: 1,
                    sessions: vec![runtime_session("runtime-concurrent", chrono::Utc::now())],
                },
            )
        });

        barrier.wait();
        std::thread::sleep(std::time::Duration::from_millis(50));
        blocker_tx.commit().unwrap();

        handle.join().unwrap().unwrap();
    }

    #[test]
    fn runtime_observation_links_only_to_resolved_canonical_session() {
        let mut store = test_store();
        store
            .connection()
            .execute(
                "INSERT INTO sessions
                 (id, provider_id, provider_session_id, status)
                 VALUES ('canonical-1', 'generic', 'provider-runtime-1', 'active')",
                [],
            )
            .unwrap();
        let now = chrono::Utc::now();
        save_runtime_sessions_in(
            store.connection_mut(),
            &RuntimeSessionStore {
                version: 1,
                sessions: vec![
                    runtime_session("runtime-1", now),
                    runtime_session("runtime-unresolved", now),
                ],
            },
        )
        .unwrap();

        let resolved: Option<String> = store
            .connection()
            .query_row(
                "SELECT session_id FROM runtime_session_observations WHERE id = 'runtime-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let unresolved: Option<String> = store
            .connection()
            .query_row(
                "SELECT session_id
                 FROM runtime_session_observations
                 WHERE id = 'runtime-unresolved'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(resolved.as_deref(), Some("canonical-1"));
        assert_eq!(unresolved, None);
    }

    #[test]
    fn server_runtime_roundtrips_token_and_index_fields() {
        let store = test_store();
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "secret-token".to_string(),
            pid: 42,
            started_at: chrono::Utc::now(),
        };

        save_server_runtime_in(store.connection(), &endpoint).unwrap();

        assert_eq!(
            load_server_runtime_in(store.connection()).unwrap(),
            Some(endpoint)
        );
        let indexed: (String, i64, String) = store
            .connection()
            .query_row(
                "SELECT host, port, token_hash FROM runtime_endpoints WHERE id = 'hook-server'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(indexed.0, "127.0.0.1");
        assert_eq!(indexed.1, 3737);
        assert_ne!(indexed.2, "secret-token");
    }

    #[test]
    fn hook_storage_does_not_create_legacy_files() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("memorph.db");
        let mut store = LocalSqliteStore::open(&database).unwrap();
        let now = chrono::Utc::now();
        let event = HookEvent::new("generic", HookEventType::Heartbeat, json!(null));

        append_event_in(store.connection_mut(), &event).unwrap();
        append_error_in(store.connection(), "test".to_string(), "error".to_string()).unwrap();
        save_runtime_sessions_in(
            store.connection_mut(),
            &RuntimeSessionStore {
                version: 1,
                sessions: vec![runtime_session("runtime-1", now)],
            },
        )
        .unwrap();

        assert!(database.exists());
        assert!(!dir.path().join("hooks").exists());
    }

    #[test]
    fn schema_contains_hook_storage_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        for table in [
            "runtime_endpoints",
            "runtime_session_observations",
            "hook_events",
            "hook_errors",
        ] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                     )",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {table}");
        }
    }
}
