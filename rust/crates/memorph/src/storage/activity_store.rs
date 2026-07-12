use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

const DEFAULT_QUERY_LIMIT: usize = 100;
const MAX_QUERY_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityActor {
    Api,
    Cli,
    Tui,
    Sync,
    System,
}

impl ActivityActor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Cli => "cli",
            Self::Tui => "tui",
            Self::Sync => "sync",
            Self::System => "system",
        }
    }
}

impl fmt::Display for ActivityActor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ActivityActor {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "api" => Ok(Self::Api),
            "cli" => Ok(Self::Cli),
            "tui" => Ok(Self::Tui),
            "sync" => Ok(Self::Sync),
            "system" => Ok(Self::System),
            _ => anyhow::bail!("Unknown activity actor: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityOperationKind {
    Scan,
    Import,
    Export,
    Sync,
    Delete,
    Rename,
    Hide,
    Pin,
    LocalStateUpdate,
    Compress,
    Backup,
    ArtifactCleanup,
}

impl ActivityOperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Import => "import",
            Self::Export => "export",
            Self::Sync => "sync",
            Self::Delete => "delete",
            Self::Rename => "rename",
            Self::Hide => "hide",
            Self::Pin => "pin",
            Self::LocalStateUpdate => "local_state_update",
            Self::Compress => "compress",
            Self::Backup => "backup",
            Self::ArtifactCleanup => "artifact_cleanup",
        }
    }
}

impl fmt::Display for ActivityOperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ActivityOperationKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "scan" => Ok(Self::Scan),
            "import" => Ok(Self::Import),
            "export" => Ok(Self::Export),
            "sync" => Ok(Self::Sync),
            "delete" => Ok(Self::Delete),
            "rename" => Ok(Self::Rename),
            "hide" => Ok(Self::Hide),
            "pin" => Ok(Self::Pin),
            "local_state_update" => Ok(Self::LocalStateUpdate),
            "compress" => Ok(Self::Compress),
            "backup" => Ok(Self::Backup),
            "artifact_cleanup" => Ok(Self::ArtifactCleanup),
            _ => anyhow::bail!("Unknown activity operation: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Running,
    Success,
    Failed,
    Partial,
}

impl ActivityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

impl fmt::Display for ActivityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ActivityStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "partial" => Ok(Self::Partial),
            _ => anyhow::bail!("Unknown activity status: {value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewActivity {
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub operation_kind: ActivityOperationKind,
    pub actor: ActivityActor,
    pub summary: String,
    pub details: Value,
}

#[derive(Debug, Clone)]
pub struct ActivityCompletion {
    pub status: ActivityStatus,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub summary: String,
    pub details: Value,
    pub error: Option<String>,
}

impl ActivityCompletion {
    pub fn success(summary: impl Into<String>, details: Value) -> Self {
        Self {
            status: ActivityStatus::Success,
            provider_id: None,
            provider_session_id: None,
            workspace_dir: None,
            summary: summary.into(),
            details,
            error: None,
        }
    }

    pub fn failed(summary: impl Into<String>, details: Value, error: impl Into<String>) -> Self {
        Self {
            status: ActivityStatus::Failed,
            provider_id: None,
            provider_session_id: None,
            workspace_dir: None,
            summary: summary.into(),
            details,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActivityQuery {
    pub session_id: Option<String>,
    pub provider_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub operation_kind: Option<ActivityOperationKind>,
    pub status: Option<ActivityStatus>,
    pub actor: Option<ActivityActor>,
    pub started_after_ms: Option<i64>,
    pub started_before_ms: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub operation_kind: ActivityOperationKind,
    pub status: ActivityStatus,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub actor: ActivityActor,
    pub summary: String,
    pub details: Value,
    pub error: Option<String>,
}

pub struct ActivityStore<'a> {
    conn: &'a Connection,
}

impl<'a> ActivityStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn start(&self, activity: NewActivity) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let started_at_ms = Utc::now().timestamp_millis();
        let session_id = resolve_canonical_session_id(
            self.conn,
            activity.provider_id.as_deref(),
            activity.provider_session_id.as_deref(),
        )?;
        let details_json = serde_json::to_string(&activity.details)
            .context("Failed to encode activity details")?;
        self.conn
            .execute(
                "INSERT INTO session_activity
                 (id, session_id, provider_id, workspace_dir, operation_kind, status,
                  started_at_ms, finished_at_ms, actor, summary, details_json, error,
                  provider_session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, NULL, ?7, ?8, ?9, NULL, ?10)",
                params![
                    id,
                    session_id,
                    activity.provider_id,
                    activity.workspace_dir,
                    activity.operation_kind.as_str(),
                    started_at_ms,
                    activity.actor.as_str(),
                    activity.summary,
                    details_json,
                    activity.provider_session_id,
                ],
            )
            .context("Failed to start session activity")?;
        Ok(id)
    }

    pub fn finish(&self, id: &str, completion: ActivityCompletion) -> Result<()> {
        if !completion.status.is_terminal() {
            anyhow::bail!("Activity completion status must be terminal");
        }

        let current = self
            .conn
            .query_row(
                "SELECT provider_id, provider_session_id, workspace_dir
                 FROM session_activity
                 WHERE id = ?1 AND status = 'running'",
                [id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .context("Failed to load running session activity")?
            .with_context(|| format!("Running session activity not found: {id}"))?;
        let provider_id = completion.provider_id.or(current.0);
        let provider_session_id = completion.provider_session_id.or(current.1);
        let workspace_dir = completion.workspace_dir.or(current.2);
        let session_id = resolve_canonical_session_id(
            self.conn,
            provider_id.as_deref(),
            provider_session_id.as_deref(),
        )?;
        let details_json = serde_json::to_string(&completion.details)
            .context("Failed to encode completed activity details")?;
        let finished_at_ms = Utc::now().timestamp_millis();
        let updated = self
            .conn
            .execute(
                "UPDATE session_activity
                 SET session_id = ?2,
                     provider_id = ?3,
                     provider_session_id = ?4,
                     workspace_dir = ?5,
                     status = ?6,
                     finished_at_ms = ?7,
                     summary = ?8,
                     details_json = ?9,
                     error = ?10
                 WHERE id = ?1 AND status = 'running'",
                params![
                    id,
                    session_id,
                    provider_id,
                    provider_session_id,
                    workspace_dir,
                    completion.status.as_str(),
                    finished_at_ms,
                    completion.summary,
                    details_json,
                    completion.error,
                ],
            )
            .context("Failed to finish session activity")?;
        if updated != 1 {
            anyhow::bail!("Running session activity not found: {id}");
        }
        Ok(())
    }

    pub fn query(&self, query: &ActivityQuery) -> Result<Vec<ActivityRecord>> {
        let mut sql = String::from(
            "SELECT id, session_id, provider_id, provider_session_id, workspace_dir,
                    operation_kind, status, started_at_ms, finished_at_ms, actor,
                    summary, details_json, error
             FROM session_activity
             WHERE 1 = 1",
        );
        let mut values = Vec::<SqlValue>::new();

        if let Some(session_id) = normalized_filter(query.session_id.as_deref()) {
            sql.push_str(" AND (session_id = ? OR provider_session_id = ?)");
            values.push(SqlValue::Text(session_id.to_string()));
            values.push(SqlValue::Text(session_id.to_string()));
        }
        if let Some(provider_id) = normalized_filter(query.provider_id.as_deref()) {
            sql.push_str(" AND provider_id = ?");
            values.push(SqlValue::Text(provider_id.to_string()));
        }
        if let Some(workspace_dir) = normalized_filter(query.workspace_dir.as_deref()) {
            sql.push_str(" AND workspace_dir = ?");
            values.push(SqlValue::Text(workspace_dir.to_string()));
        }
        if let Some(operation_kind) = query.operation_kind {
            sql.push_str(" AND operation_kind = ?");
            values.push(SqlValue::Text(operation_kind.as_str().to_string()));
        }
        if let Some(status) = query.status {
            sql.push_str(" AND status = ?");
            values.push(SqlValue::Text(status.as_str().to_string()));
        }
        if let Some(actor) = query.actor {
            sql.push_str(" AND actor = ?");
            values.push(SqlValue::Text(actor.as_str().to_string()));
        }
        if let Some(started_after_ms) = query.started_after_ms {
            sql.push_str(" AND started_at_ms >= ?");
            values.push(SqlValue::Integer(started_after_ms));
        }
        if let Some(started_before_ms) = query.started_before_ms {
            sql.push_str(" AND started_at_ms <= ?");
            values.push(SqlValue::Integer(started_before_ms));
        }

        sql.push_str(" ORDER BY started_at_ms DESC, id DESC LIMIT ?");
        values.push(SqlValue::Integer(
            query
                .limit
                .unwrap_or(DEFAULT_QUERY_LIMIT)
                .min(MAX_QUERY_LIMIT) as i64,
        ));

        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("Failed to prepare session activity query")?;
        let rows = stmt
            .query_map(params_from_iter(values), decode_activity_record)
            .context("Failed to query session activity")?;
        let mut activities = Vec::new();
        for row in rows {
            activities.push(row.context("Failed to decode session activity")?);
        }
        Ok(activities)
    }
}

fn normalized_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn resolve_canonical_session_id(
    conn: &Connection,
    provider_id: Option<&str>,
    provider_session_id: Option<&str>,
) -> Result<Option<String>> {
    let (Some(provider_id), Some(provider_session_id)) = (
        normalized_filter(provider_id),
        normalized_filter(provider_session_id),
    ) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT id
         FROM sessions
         WHERE provider_id = ?1 AND provider_session_id = ?2
         UNION
         SELECT session_id
         FROM session_aliases
         WHERE provider_id = ?1 AND alias_value = ?2
         LIMIT 1",
        params![provider_id, provider_session_id],
        |row| row.get(0),
    )
    .optional()
    .context("Failed to resolve canonical session activity identity")
}

fn decode_activity_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityRecord> {
    let operation_kind_text = row.get::<_, String>(5)?;
    let status_text = row.get::<_, String>(6)?;
    let actor_text = row.get::<_, String>(9)?;
    let details_json = row.get::<_, String>(11)?;
    let operation_kind =
        ActivityOperationKind::from_str(&operation_kind_text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                operation_kind_text.len(),
                rusqlite::types::Type::Text,
                error.into(),
            )
        })?;
    let status = ActivityStatus::from_str(&status_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            status_text.len(),
            rusqlite::types::Type::Text,
            error.into(),
        )
    })?;
    let actor = ActivityActor::from_str(&actor_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            actor_text.len(),
            rusqlite::types::Type::Text,
            error.into(),
        )
    })?;
    let details = serde_json::from_str(&details_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            details_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(ActivityRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        provider_id: row.get(2)?,
        provider_session_id: row.get(3)?,
        workspace_dir: row.get(4)?,
        operation_kind,
        status,
        started_at_ms: row.get(7)?,
        finished_at_ms: row.get(8)?,
        actor,
        summary: row.get(10)?,
        details,
        error: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store;

    fn test_connection() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn
    }

    #[test]
    fn records_success_and_failure_with_native_and_canonical_identity() {
        let conn = test_connection();
        seed_session(&conn);
        let store = ActivityStore::new(&conn);
        let success_id = store
            .start(NewActivity {
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("native-1".to_string()),
                workspace_dir: Some("/tmp/project".to_string()),
                operation_kind: ActivityOperationKind::Export,
                actor: ActivityActor::Cli,
                summary: "Exporting session".to_string(),
                details: serde_json::json!({"format": "json"}),
            })
            .unwrap();
        store
            .finish(
                &success_id,
                ActivityCompletion::success(
                    "Exported session",
                    serde_json::json!({"files": ["/tmp/session.json"]}),
                ),
            )
            .unwrap();

        let failed_id = store
            .start(NewActivity {
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("missing".to_string()),
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Delete,
                actor: ActivityActor::Api,
                summary: "Deleting session".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();
        store
            .finish(
                &failed_id,
                ActivityCompletion::failed(
                    "Failed to delete session",
                    serde_json::json!({"provider_session_id": "missing"}),
                    "not found",
                ),
            )
            .unwrap();

        let activities = store.query(&ActivityQuery::default()).unwrap();
        assert_eq!(activities.len(), 2);
        let success = activities
            .iter()
            .find(|activity| activity.id == success_id)
            .unwrap();
        assert_eq!(success.session_id.as_deref(), Some("canonical-1"));
        assert_eq!(success.status, ActivityStatus::Success);
        assert!(success.finished_at_ms.is_some());
        let failed = activities
            .iter()
            .find(|activity| activity.id == failed_id)
            .unwrap();
        assert_eq!(failed.session_id, None);
        assert_eq!(failed.provider_session_id.as_deref(), Some("missing"));
        assert_eq!(failed.error.as_deref(), Some("not found"));
    }

    #[test]
    fn completion_can_attach_provider_identity_and_resolve_canonical_session() {
        let conn = test_connection();
        seed_session(&conn);
        let store = ActivityStore::new(&conn);
        let activity_id = store
            .start(NewActivity {
                provider_id: None,
                provider_session_id: None,
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Sync,
                actor: ActivityActor::Api,
                summary: "Synchronizing session".to_string(),
                details: serde_json::json!({"group_id": "group-1"}),
            })
            .unwrap();

        store
            .finish(
                &activity_id,
                ActivityCompletion {
                    status: ActivityStatus::Success,
                    provider_id: Some("claude".to_string()),
                    provider_session_id: Some("native-1".to_string()),
                    workspace_dir: Some("/tmp/project".to_string()),
                    summary: "Synchronized session".to_string(),
                    details: serde_json::json!({"success": ["codex"]}),
                    error: None,
                },
            )
            .unwrap();

        let activity = store
            .query(&ActivityQuery {
                session_id: Some("canonical-1".to_string()),
                ..ActivityQuery::default()
            })
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(activity.provider_id.as_deref(), Some("claude"));
        assert_eq!(activity.provider_session_id.as_deref(), Some("native-1"));
        assert_eq!(activity.session_id.as_deref(), Some("canonical-1"));
        assert_eq!(activity.workspace_dir.as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn queries_by_session_provider_workspace_operation_status_actor_and_time() {
        let conn = test_connection();
        seed_session(&conn);
        let store = ActivityStore::new(&conn);
        let id = store
            .start(NewActivity {
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("native-1".to_string()),
                workspace_dir: Some("/tmp/project".to_string()),
                operation_kind: ActivityOperationKind::Rename,
                actor: ActivityActor::Tui,
                summary: "Renaming session".to_string(),
                details: serde_json::json!({"title": "New title"}),
            })
            .unwrap();
        store
            .finish(
                &id,
                ActivityCompletion::success(
                    "Renamed session",
                    serde_json::json!({"title": "New title"}),
                ),
            )
            .unwrap();

        let activities = store
            .query(&ActivityQuery {
                session_id: Some("native-1".to_string()),
                provider_id: Some("claude".to_string()),
                workspace_dir: Some("/tmp/project".to_string()),
                operation_kind: Some(ActivityOperationKind::Rename),
                status: Some(ActivityStatus::Success),
                actor: Some(ActivityActor::Tui),
                started_after_ms: Some(0),
                started_before_ms: Some(i64::MAX),
                limit: Some(1),
            })
            .unwrap();

        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].id, id);
    }

    #[test]
    fn rejects_finishing_an_activity_twice() {
        let conn = test_connection();
        let store = ActivityStore::new(&conn);
        let id = store
            .start(NewActivity {
                provider_id: None,
                provider_session_id: None,
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Scan,
                actor: ActivityActor::System,
                summary: "Scanning".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();
        store
            .finish(
                &id,
                ActivityCompletion::success("Scanned", serde_json::json!({})),
            )
            .unwrap();

        let error = store
            .finish(
                &id,
                ActivityCompletion::success("Scanned again", serde_json::json!({})),
            )
            .unwrap_err();

        assert!(error.to_string().contains("not found"));
    }

    fn seed_session(conn: &Connection) {
        conn.execute(
            "INSERT INTO sessions (id, provider_id, provider_session_id)
             VALUES ('canonical-1', 'claude', 'native-1')",
            [],
        )
        .unwrap();
    }
}
