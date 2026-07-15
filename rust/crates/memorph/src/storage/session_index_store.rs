use crate::provider::{ProviderCapabilities, ProviderSessionSummary, StorageShape};
use crate::session_projection::{SessionIdentity, SessionIdentityInput};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::{Path, PathBuf};

const SESSION_INDEX_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSessionIndex {
    pub canonical_session_id: String,
    pub source_id: String,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub modified_at_ms: i64,
    pub size_bytes: i64,
    pub value: String,
}

pub struct SessionIndexStore<'a> {
    conn: &'a mut Connection,
}

impl<'a> SessionIndexStore<'a> {
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    pub fn write_session_summary(
        &mut self,
        provider_id: &str,
        summary: &ProviderSessionSummary,
        capabilities: ProviderCapabilities,
    ) -> Result<StoredSessionIndex> {
        let source_path = summary
            .source_path
            .as_deref()
            .filter(|value| !value.is_empty())
            .context("Provider session summary has no source path")?;
        let fingerprint = source_fingerprint(Path::new(source_path))?.with_context(|| {
            format!(
                "Session source does not exist: {}",
                source_file_path(Path::new(source_path)).display()
            )
        })?;
        let identity = SessionIdentity::from_source(SessionIdentityInput {
            provider_id,
            provider_session_id: Some(&summary.session_id),
            source_path: Some(source_path),
            workspace_dir: summary.project_dir.as_deref(),
        })?;
        let canonical_session_id = identity.canonical_session_id.clone();
        let source_id = stable_row_id("source", provider_id, source_path);
        let title = summary
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&summary.session_id);
        let now_ms = Utc::now().timestamp_millis();
        let tx = self
            .conn
            .transaction()
            .context("Failed to start session index transaction")?;

        tx.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir, storage_shape,
              file_mtime_ms, file_size_bytes, content_hash, source_cursor, scan_generation,
              provider_schema_version, first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, 1, NULL, ?10, ?10)
             ON CONFLICT(provider_id, source_path) DO UPDATE SET
              provider_session_id = excluded.provider_session_id,
              workspace_dir = excluded.workspace_dir,
              storage_shape = excluded.storage_shape,
              file_mtime_ms = excluded.file_mtime_ms,
              file_size_bytes = excluded.file_size_bytes,
              content_hash = NULL,
              source_cursor = excluded.source_cursor,
              scan_generation = session_sources.scan_generation + 1,
              provider_schema_version = NULL,
              last_seen_at_ms = excluded.last_seen_at_ms",
            params![
                source_id,
                provider_id,
                summary.session_id,
                source_path,
                summary.project_dir.as_deref(),
                storage_shape_name(capabilities.storage_shape),
                fingerprint.modified_at_ms,
                fingerprint.size_bytes,
                fingerprint.value,
                now_ms,
            ],
        )
        .context("Failed to upsert session source index")?;

        tx.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title, status,
              created_at_ms, updated_at_ms, last_active_at_ms, event_count, turn_count,
              projection_version, deleted_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'indexed', NULL, ?7, ?8, 0, 0, ?9, NULL)
             ON CONFLICT(id) DO UPDATE SET
              provider_id = excluded.provider_id,
              provider_session_id = excluded.provider_session_id,
              primary_source_id = excluded.primary_source_id,
              workspace_dir = excluded.workspace_dir,
              title = excluded.title,
              status = excluded.status,
              updated_at_ms = excluded.updated_at_ms,
              last_active_at_ms = excluded.last_active_at_ms,
              projection_version = excluded.projection_version,
              deleted_at_ms = NULL",
            params![
                canonical_session_id,
                provider_id,
                summary.session_id,
                source_id,
                summary.project_dir.as_deref(),
                title,
                now_ms,
                summary.last_active_at,
                SESSION_INDEX_VERSION,
            ],
        )
        .context("Failed to upsert session identity index")?;

        for alias in &identity.aliases {
            tx.execute(
                "INSERT INTO session_aliases
                 (alias_kind, alias_value, session_id, provider_id, source_id, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(alias_kind, alias_value) DO UPDATE SET
                  session_id = excluded.session_id,
                  provider_id = excluded.provider_id,
                  source_id = excluded.source_id",
                params![
                    session_alias_kind_name(alias.kind),
                    alias.value,
                    canonical_session_id,
                    alias.provider_id.as_deref(),
                    source_id,
                    now_ms,
                ],
            )
            .context("Failed to upsert session alias index")?;
        }

        let snapshot_json = json!({
            "index_version": SESSION_INDEX_VERSION,
            "source_fingerprint": fingerprint.value,
        });
        tx.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, display_title, workspace_dir, status,
              last_active_at_ms, event_count, turn_count, flags_json, snapshot_json,
              projection_version, source_fingerprint, stale, updated_at_ms,
              message_count, counts_complete)
             VALUES (?1, ?2, ?3, NULL, ?4, 'indexed', ?5, 0, 0, '{}', ?6, ?7, ?8, 0, ?9,
                     NULL, 0)
             ON CONFLICT(session_id) DO UPDATE SET
              provider_id = excluded.provider_id,
              title = excluded.title,
              workspace_dir = excluded.workspace_dir,
              status = excluded.status,
              last_active_at_ms = excluded.last_active_at_ms,
              event_count = CASE
                  WHEN session_snapshots.source_fingerprint = excluded.source_fingerprint
                  THEN session_snapshots.event_count ELSE 0 END,
              turn_count = CASE
                  WHEN session_snapshots.source_fingerprint = excluded.source_fingerprint
                  THEN session_snapshots.turn_count ELSE 0 END,
              message_count = CASE
                  WHEN session_snapshots.source_fingerprint = excluded.source_fingerprint
                  THEN session_snapshots.message_count ELSE NULL END,
              counts_complete = CASE
                  WHEN session_snapshots.source_fingerprint = excluded.source_fingerprint
                  THEN session_snapshots.counts_complete ELSE 0 END,
              snapshot_json = excluded.snapshot_json,
              projection_version = excluded.projection_version,
              source_fingerprint = excluded.source_fingerprint,
              stale = 0,
              updated_at_ms = excluded.updated_at_ms",
            params![
                canonical_session_id,
                provider_id,
                title,
                summary.project_dir.as_deref(),
                summary.last_active_at,
                snapshot_json.to_string(),
                SESSION_INDEX_VERSION,
                fingerprint.value,
                now_ms,
            ],
        )
        .context("Failed to upsert session list snapshot")?;

        tx.commit()
            .context("Failed to commit session index transaction")?;

        Ok(StoredSessionIndex {
            canonical_session_id,
            source_id,
            source_fingerprint: fingerprint.value,
        })
    }

    pub fn record_complete_counts(
        &mut self,
        canonical_session_id: &str,
        expected_source_fingerprint: &str,
        event_count: usize,
        message_count: usize,
        turn_count: usize,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE session_snapshots
             SET event_count = ?3,
                 message_count = ?4,
                 turn_count = ?5,
                 counts_complete = 1,
                 updated_at_ms = ?6
             WHERE session_id = ?1
               AND source_fingerprint = ?2",
            params![
                canonical_session_id,
                expected_source_fingerprint,
                event_count as i64,
                message_count as i64,
                turn_count as i64,
                Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(changed == 1)
    }
}

pub fn source_file_path(path: &Path) -> PathBuf {
    let path_text = path.to_string_lossy();
    let file_path = match path_text.split_once('#') {
        Some((file_path, fragment)) if fragment.starts_with("session=") => file_path,
        _ => path_text.as_ref(),
    };
    PathBuf::from(file_path)
}

pub fn source_fingerprint(path: &Path) -> Result<Option<SourceFingerprint>> {
    let file_path = source_file_path(path);
    let metadata = match std::fs::metadata(&file_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read session source metadata: {}",
                    file_path.display()
                )
            })
        }
    };
    let mut modified_at_ms = metadata_modified_ms(&metadata);
    let mut size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    let mut wal_modified_at_ms = 0;
    let mut wal_size_bytes = 0;

    if file_path.extension().and_then(|value| value.to_str()) == Some("db")
        || path.to_string_lossy().contains("#session=")
    {
        let wal_path = PathBuf::from(format!("{}-wal", file_path.to_string_lossy()));
        if let Ok(wal_metadata) = std::fs::metadata(&wal_path) {
            wal_modified_at_ms = metadata_modified_ms(&wal_metadata);
            wal_size_bytes = i64::try_from(wal_metadata.len()).unwrap_or(i64::MAX);
            modified_at_ms = modified_at_ms.max(wal_modified_at_ms);
            size_bytes = size_bytes.saturating_add(wal_size_bytes);
        }
    }

    Ok(Some(SourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "metadata-v1:{modified_at_ms}:{size_bytes}:{wal_modified_at_ms}:{wal_size_bytes}"
        ),
    }))
}

fn metadata_modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn storage_shape_name(shape: StorageShape) -> &'static str {
    match shape {
        StorageShape::Unknown => "unknown",
        StorageShape::Jsonl => "jsonl",
        StorageShape::Sqlite => "sqlite",
        StorageShape::Directory => "directory",
        StorageShape::Mixed => "mixed",
    }
}

fn session_alias_kind_name(kind: crate::session_projection::SessionAliasKind) -> &'static str {
    match kind {
        crate::session_projection::SessionAliasKind::ProviderSessionId => "provider_session_id",
        crate::session_projection::SessionAliasKind::SourcePath => "source_path",
        crate::session_projection::SessionAliasKind::SyncHoldingId => "sync_holding_id",
        crate::session_projection::SessionAliasKind::HookCorrelationId => "hook_correlation_id",
    }
}

fn stable_row_id(kind: &str, left: &str, right: &str) -> String {
    format!(
        "{}_{:x}",
        kind,
        md5::compute(format!("{}\0{}", left, right).as_bytes())
    )
}
