//! Hook event, error, and endpoint persistence.
//!
//! Provider-native session files remain provider-owned. Hook events, errors,
//! and the local hook endpoint are memorph management data stored in SQLite;
//! lightweight runtime sessions remain in memory.

use anyhow::{Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{OnceLock, RwLock};

use crate::hooks::model::HookEvent;
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
pub struct HookErrorRecord {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub scope: String,
    pub message: String,
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
    tx.execute(
        "INSERT INTO hook_events
         (id, provider_id, provider_session_id, session_id, event_name, observed_at_ms,
          correlation_id, payload_json)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7)",
        params![
            event.event_id,
            event.provider,
            event.provider_session_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::HookEventType;
    use crate::storage::local_store;
    use serde_json::{json, Value};

    fn test_store() -> LocalSqliteStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep().join("memorph.db");
        LocalSqliteStore::open(path).unwrap()
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
        for table in ["hook_errors", "session_activity"] {
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
        let event = HookEvent::new("generic", HookEventType::Heartbeat, json!(null));

        append_event_in(store.connection_mut(), &event).unwrap();
        append_error_in(store.connection(), "test".to_string(), "error".to_string()).unwrap();

        assert!(database.exists());
        assert!(!dir.path().join("hooks").exists());
    }

    #[test]
    fn schema_contains_hook_storage_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        for table in ["runtime_endpoints", "hook_events", "hook_errors"] {
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
        let legacy_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runtime_session_observations'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !legacy_exists,
            "legacy runtime observation table should be removed"
        );
    }
}
