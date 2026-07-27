use anyhow::{Context as _, Result};
use rusqlite::{params, params_from_iter, types::Value, Connection};
use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotStaleScanReport {
    pub checked_sources: usize,
    pub fresh_snapshots: usize,
    pub stale_snapshots: usize,
    pub missing_sources: usize,
    pub unknown_sources: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleSnapshotSourceRow {
    pub canonical_session_id: String,
    pub provider_id: String,
    pub provider_session_id: Option<String>,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedSessionSnapshotRow {
    pub canonical_session_id: String,
    pub provider_id: String,
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    pub workspace_dir: Option<String>,
    pub last_active_at_ms: Option<i64>,
    pub source_path: Option<String>,
    pub message_count: Option<usize>,
    pub event_count: usize,
    pub turn_count: usize,
    pub size_bytes: Option<u64>,
    pub hidden: bool,
    pub pinned: bool,
    pub preferred_targets: Vec<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSessionSummaryRow {
    pub path: String,
    pub session_count: usize,
    pub last_active_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSessionPageRow {
    pub items: Vec<WorkspaceSessionSummaryRow>,
    pub total_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedSessionIdentityRow {
    pub canonical_session_id: String,
    pub source_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    pub workspace_dir: Option<String>,
    pub source_path: Option<String>,
    pub source_fingerprint: Option<String>,
    pub last_active_at_ms: Option<i64>,
    pub stale: bool,
}

pub struct SnapshotStore<'a> {
    conn: &'a Connection,
}

impl<'a> SnapshotStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn refresh_session_snapshot_staleness<F>(
        &self,
        mut source_fingerprint: F,
    ) -> Result<SnapshotStaleScanReport>
    where
        F: FnMut(&str, &str) -> Result<Option<String>>,
    {
        let rows = self.session_snapshot_sources()?;
        let mut report = SnapshotStaleScanReport::default();
        for row in rows {
            let Some(source_path) = row.source_path.as_deref().filter(|value| !value.is_empty())
            else {
                report.unknown_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };
            let Some(stored_fingerprint) = row
                .source_fingerprint
                .as_deref()
                .filter(|value| !value.is_empty())
            else {
                report.unknown_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };

            let Some(current_fingerprint) = source_fingerprint(&row.provider_id, source_path)?
            else {
                report.missing_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };

            report.checked_sources += 1;
            let stale = current_fingerprint != stored_fingerprint;
            if stale {
                report.stale_snapshots += 1;
            } else {
                report.fresh_snapshots += 1;
            }
            self.set_session_snapshot_stale(&row.session_id, stale)?;
        }
        Ok(report)
    }

    pub fn list_session_snapshots(&self) -> Result<Vec<ProjectedSessionSnapshotRow>> {
        self.list_session_snapshots_filtered(None, None, true)
    }

    pub fn list_workspaces_with_sessions(
        &self,
        search: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<WorkspaceSessionPageRow> {
        let page_size = page_size.max(1);
        let search = search.map(str::to_lowercase);
        let search_filter = "(?1 IS NULL OR instr(lower(rtrim(ss.workspace_dir, '/\\')), ?1) > 0)";
        let base_filter = format!(
            "s.deleted_at_ms IS NULL
             AND ss.workspace_dir IS NOT NULL
             AND trim(rtrim(ss.workspace_dir, '/\\')) NOT IN ('', '—', '-')
             AND {search_filter}"
        );

        let total_sql = format!(
            "SELECT COUNT(*) FROM (
                SELECT rtrim(ss.workspace_dir, '/\\')
                FROM session_snapshots ss
                JOIN sessions s ON s.id = ss.session_id
                WHERE {base_filter}
                GROUP BY rtrim(ss.workspace_dir, '/\\')
            )"
        );
        let total_count: usize = self
            .conn
            .query_row(&total_sql, [search.as_deref()], |row| {
                row.get::<_, i64>(0).map(|value| value.max(0) as usize)
            })
            .context("Failed to count workspaces with sessions")?;

        let total_pages = total_count.div_ceil(page_size).max(1);
        let page = page.max(1).min(total_pages);
        let offset = (page - 1) * page_size;

        let page_sql = format!(
            "SELECT
                rtrim(ss.workspace_dir, '/\\') AS path,
                COUNT(*) AS session_count,
                MAX(ss.last_active_at_ms) AS last_active_at_ms
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             WHERE {base_filter}
             GROUP BY rtrim(ss.workspace_dir, '/\\')
             ORDER BY last_active_at_ms DESC, lower(path) ASC
             LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self
            .conn
            .prepare(&page_sql)
            .context("Failed to prepare workspace session page query")?;
        let rows = stmt
            .query_map(
                rusqlite::params![search.as_deref(), page_size as i64, offset as i64],
                |row| {
                    Ok(WorkspaceSessionSummaryRow {
                        path: row.get(0)?,
                        session_count: row.get::<_, i64>(1)?.max(0) as usize,
                        last_active_at_ms: row.get(2)?,
                    })
                },
            )
            .context("Failed to query workspace session page")?;
        let items = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to decode workspace session page")?;

        Ok(WorkspaceSessionPageRow { items, total_count })
    }

    pub fn list_session_snapshots_filtered(
        &self,
        provider_ids: Option<&[String]>,
        workspace_scopes: Option<&[(String, String)]>,
        include_message_counts: bool,
    ) -> Result<Vec<ProjectedSessionSnapshotRow>> {
        let message_count_sql = if include_message_counts {
            "CASE WHEN ss.counts_complete = 1 THEN ss.message_count ELSE NULL END"
        } else {
            "NULL"
        };
        let mut where_clauses = vec!["s.deleted_at_ms IS NULL".to_string()];
        let mut values = Vec::new();

        if let Some(provider_ids) = provider_ids.filter(|ids| !ids.is_empty()) {
            let placeholders = vec!["?"; provider_ids.len()].join(", ");
            where_clauses.push(format!("ss.provider_id IN ({placeholders})"));
            values.extend(provider_ids.iter().cloned().map(Value::Text));
        }

        if let Some(workspace_scopes) = workspace_scopes.filter(|scopes| !scopes.is_empty()) {
            let scopes = workspace_scopes
                .iter()
                .map(|_| "(ss.provider_id = ? AND ss.workspace_dir = ?)")
                .collect::<Vec<_>>()
                .join(" OR ");
            where_clauses.push(format!("({scopes})"));
            for (provider_id, workspace_dir) in workspace_scopes {
                values.push(Value::Text(provider_id.clone()));
                values.push(Value::Text(workspace_dir.clone()));
            }
        }

        let sql = format!(
            "SELECT
                ss.session_id,
                ss.provider_id,
                COALESCE(s.provider_session_id, src.provider_session_id),
                ss.title,
                COALESCE(local.display_title, ss.display_title),
                ss.workspace_dir,
                ss.last_active_at_ms,
                src.source_path,
                {message_count_sql},
                COALESCE(ss.event_count, s.event_count, 0),
                COALESCE(ss.turn_count, s.turn_count, 0),
                CASE WHEN src.storage_shape = 'sqlite' OR instr(src.source_path, '#') > 0 THEN NULL ELSE src.file_size_bytes END,
                ss.flags_json,
                COALESCE(workspace.hidden, local.hidden),
                COALESCE(workspace.pinned, local.pinned),
                COALESCE(workspace.preferred_targets_json, local.preferred_targets_json),
                ss.stale
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             LEFT JOIN session_sources src ON src.id = s.primary_source_id
             LEFT JOIN session_local_state local ON local.session_id = ss.session_id
             LEFT JOIN workspace_session_state workspace
                ON workspace.session_id = ss.session_id
               AND workspace.workspace_dir = ss.workspace_dir
             WHERE {}
             ORDER BY
                ss.provider_id ASC,
                COALESCE(workspace.pinned, local.pinned, 0) DESC,
                ss.last_active_at_ms DESC,
                lower(COALESCE(local.display_title, ss.display_title, ss.title, ss.session_id)) ASC",
            where_clauses.join(" AND ")
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("Failed to prepare projected session snapshot list")?;

        let rows = stmt
            .query_map(params_from_iter(values.iter()), |row| {
                let flags_json: String = row.get(12)?;
                let local_hidden: Option<i64> = row.get(13)?;
                let local_pinned: Option<i64> = row.get(14)?;
                let preferred_targets_json: Option<String> = row.get(15)?;
                let flags = parse_flags(&flags_json);
                let file_size_bytes: Option<i64> = row.get(11)?;

                Ok(ProjectedSessionSnapshotRow {
                    canonical_session_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    provider_session_id: row.get(2)?,
                    title: row.get(3)?,
                    display_title: row.get(4)?,
                    workspace_dir: row.get(5)?,
                    last_active_at_ms: row.get(6)?,
                    source_path: row.get(7)?,
                    message_count: row
                        .get::<_, Option<i64>>(8)?
                        .map(|value| value.max(0) as usize),
                    event_count: row.get::<_, i64>(9)?.max(0) as usize,
                    turn_count: row.get::<_, i64>(10)?.max(0) as usize,
                    size_bytes: file_size_bytes.and_then(|value| u64::try_from(value).ok()),
                    hidden: local_hidden.map(sql_bool).unwrap_or(flags.hidden),
                    pinned: local_pinned.map(sql_bool).unwrap_or(flags.pinned),
                    preferred_targets: parse_string_list(preferred_targets_json.as_deref()),
                    stale: sql_bool(row.get::<_, i64>(16)?),
                })
            })
            .context("Failed to query projected session snapshots")?;

        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(row.context("Failed to decode projected session snapshot")?);
        }
        Ok(snapshots)
    }

    pub fn find_session_identity(
        &self,
        provider_id: &str,
        session_id: &str,
    ) -> Result<Option<ProjectedSessionIdentityRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    s.id,
                    src.id,
                    COALESCE(s.provider_session_id, src.provider_session_id),
                    COALESCE(ss.title, s.title),
                    COALESCE(local.display_title, ss.display_title),
                    COALESCE(ss.workspace_dir, s.workspace_dir, src.workspace_dir),
                    src.source_path,
                    ss.source_fingerprint,
                    COALESCE(ss.last_active_at_ms, s.last_active_at_ms),
                    ss.stale
                 FROM sessions s
                 JOIN session_snapshots ss ON ss.session_id = s.id
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 LEFT JOIN session_local_state local ON local.session_id = s.id
                 WHERE s.deleted_at_ms IS NULL
                   AND s.provider_id = ?1
                   AND (
                        s.provider_session_id = ?2
                        OR src.provider_session_id = ?2
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
                 ORDER BY COALESCE(ss.last_active_at_ms, s.last_active_at_ms) DESC
                 LIMIT 1",
            )
            .context("Failed to prepare projected session identity query")?;
        let mut rows = stmt
            .query([provider_id, session_id])
            .context("Failed to query projected session identity")?;
        let Some(row) = rows
            .next()
            .context("Failed to decode projected session identity")?
        else {
            return Ok(None);
        };

        Ok(Some(ProjectedSessionIdentityRow {
            canonical_session_id: row.get(0)?,
            source_id: row.get(1)?,
            provider_session_id: row.get(2)?,
            title: row.get(3)?,
            display_title: row.get(4)?,
            workspace_dir: row.get(5)?,
            source_path: row.get(6)?,
            source_fingerprint: row.get(7)?,
            last_active_at_ms: row.get(8)?,
            stale: sql_bool(row.get(9)?),
        }))
    }

    pub fn list_provider_session_identities(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProjectedSessionIdentityRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    s.id,
                    src.id,
                    COALESCE(s.provider_session_id, src.provider_session_id),
                    COALESCE(ss.title, s.title),
                    COALESCE(local.display_title, ss.display_title),
                    COALESCE(ss.workspace_dir, s.workspace_dir, src.workspace_dir),
                    src.source_path,
                    ss.source_fingerprint,
                    COALESCE(ss.last_active_at_ms, s.last_active_at_ms),
                    ss.stale
                 FROM sessions s
                 JOIN session_snapshots ss ON ss.session_id = s.id
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 LEFT JOIN session_local_state local ON local.session_id = s.id
                 WHERE s.deleted_at_ms IS NULL
                   AND s.provider_id = ?1
                 ORDER BY
                    COALESCE(ss.last_active_at_ms, s.last_active_at_ms) DESC,
                    s.id ASC",
            )
            .context("Failed to prepare projected provider session identity list")?;
        let rows = stmt
            .query_map([provider_id], |row| {
                Ok(ProjectedSessionIdentityRow {
                    canonical_session_id: row.get(0)?,
                    source_id: row.get(1)?,
                    provider_session_id: row.get(2)?,
                    title: row.get(3)?,
                    display_title: row.get(4)?,
                    workspace_dir: row.get(5)?,
                    source_path: row.get(6)?,
                    source_fingerprint: row.get(7)?,
                    last_active_at_ms: row.get(8)?,
                    stale: sql_bool(row.get(9)?),
                })
            })
            .context("Failed to query projected provider session identities")?;

        let mut identities = Vec::new();
        for row in rows {
            identities.push(row.context("Failed to decode projected provider session identity")?);
        }
        Ok(identities)
    }

    pub fn list_stale_snapshot_sources(
        &self,
        provider_id: Option<&str>,
    ) -> Result<Vec<StaleSnapshotSourceRow>> {
        let mut sql = String::from(
            "SELECT
                ss.session_id,
                ss.provider_id,
                COALESCE(s.provider_session_id, src.provider_session_id),
                src.source_path
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             LEFT JOIN session_sources src ON src.id = s.primary_source_id
             WHERE s.deleted_at_ms IS NULL
               AND ss.stale = 1",
        );
        if provider_id.is_some() {
            sql.push_str(" AND ss.provider_id = ?1");
        }
        sql.push_str(" ORDER BY ss.provider_id, ss.session_id");

        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("Failed to prepare stale projected session sources query")?;
        let mut rows = if let Some(provider_id) = provider_id {
            stmt.query([provider_id])
                .context("Failed to query stale projected session sources")?
        } else {
            stmt.query([])
                .context("Failed to query stale projected session sources")?
        };

        let mut sources = Vec::new();
        while let Some(row) = rows
            .next()
            .context("Failed to decode stale projected session source row")?
        {
            sources.push(StaleSnapshotSourceRow {
                canonical_session_id: row.get(0)?,
                provider_id: row.get(1)?,
                provider_session_id: row.get(2)?,
                source_path: row.get(3)?,
            });
        }
        Ok(sources)
    }

    pub fn session_source_is_fresh(
        &self,
        provider_id: &str,
        provider_session_id: &str,
        source_path: &str,
        current_fingerprint: &str,
    ) -> Result<bool> {
        let exists = self
            .conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM sessions s
                    JOIN session_snapshots ss ON ss.session_id = s.id
                    JOIN session_sources src ON src.id = s.primary_source_id
                    WHERE s.deleted_at_ms IS NULL
                      AND s.provider_id = ?1
                      AND COALESCE(s.provider_session_id, src.provider_session_id) = ?2
                      AND src.source_path = ?3
                      AND ss.source_fingerprint = ?4
                 )",
                params![
                    provider_id,
                    provider_session_id,
                    source_path,
                    current_fingerprint
                ],
                |row| row.get::<_, i64>(0),
            )
            .context("Failed to query projected session source freshness")?;
        Ok(exists != 0)
    }

    fn session_snapshot_sources(&self) -> Result<Vec<SnapshotSourceRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ss.session_id, s.provider_id, src.source_path, ss.source_fingerprint
                 FROM session_snapshots ss
                 JOIN sessions s ON s.id = ss.session_id
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 WHERE s.deleted_at_ms IS NULL
                 ORDER BY ss.session_id",
            )
            .context("Failed to prepare projected session source staleness scan")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SnapshotSourceRow {
                    session_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    source_path: row.get(2)?,
                    source_fingerprint: row.get(3)?,
                })
            })
            .context("Failed to query projected session source staleness scan")?;

        let mut sources = Vec::new();
        for row in rows {
            sources.push(row.context("Failed to decode projected session source staleness row")?);
        }
        Ok(sources)
    }

    fn set_session_snapshot_stale(&self, session_id: &str, stale: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE session_snapshots SET stale = ?1 WHERE session_id = ?2",
                (if stale { 1_i64 } else { 0_i64 }, session_id),
            )
            .with_context(|| {
                format!("Failed to update projected session staleness: {session_id}")
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct SnapshotSourceRow {
    session_id: String,
    provider_id: String,
    source_path: Option<String>,
    source_fingerprint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SnapshotFlags {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    pinned: bool,
}

fn parse_flags(value: &str) -> SnapshotFlags {
    serde_json::from_str(value).unwrap_or_default()
}

fn parse_string_list(value: Option<&str>) -> Vec<String> {
    value
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn sql_bool(value: i64) -> bool {
    value != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{provider::Provider, providers::claude::ClaudeProvider, storage::local_store};
    use rusqlite::params;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn test_source_fingerprint(source_path: &str) -> Result<Option<String>> {
        Ok(ClaudeProvider
            .session_source_fingerprint(source_path)?
            .map(|fingerprint| fingerprint.value))
    }

    fn required_test_source_fingerprint(source_path: &str) -> String {
        test_source_fingerprint(source_path)
            .unwrap()
            .expect("test source fingerprint")
    }

    #[test]
    fn reads_projected_session_snapshots() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            20,
        );
        conn.execute(
            "UPDATE session_snapshots
             SET message_count = 1, counts_complete = 1
             WHERE session_id = 'canonical-1'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_local_state
             (session_id, display_title, hidden, pinned, preferred_targets_json, updated_at_ms)
             VALUES ('canonical-1', 'Local title', 1, 1, '[\"codex\"]', 30)",
            [],
        )
        .unwrap();

        let rows = SnapshotStore::new(&conn).list_session_snapshots().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_session_id, "canonical-1");
        assert_eq!(rows[0].provider_session_id.as_deref(), Some("native-1"));
        assert_eq!(rows[0].display_title.as_deref(), Some("Local title"));
        assert_eq!(rows[0].workspace_dir.as_deref(), Some("/tmp/project"));
        assert_eq!(rows[0].source_path.as_deref(), Some("/tmp/source.jsonl"));
        assert_eq!(rows[0].message_count, Some(1));
        assert_eq!(rows[0].event_count, 2);
        assert_eq!(rows[0].turn_count, 1);
        assert_eq!(rows[0].size_bytes, Some(20));
        assert!(rows[0].hidden);
        assert!(rows[0].pinned);
        assert_eq!(rows[0].preferred_targets, vec!["codex"]);
    }

    #[test]
    fn list_session_snapshots_does_not_read_provider_source_file() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let file = NamedTempFile::new().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        insert_projected_snapshot_with_source_path(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            &source_path,
            20,
        );
        drop(file);

        let rows = SnapshotStore::new(&conn).list_session_snapshots().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_path.as_deref(), Some(source_path.as_str()));
        assert_eq!(rows[0].provider_session_id.as_deref(), Some("native-1"));
    }

    #[test]
    fn finds_projected_session_identity_by_native_canonical_and_alias_ids() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            20,
        );
        conn.execute(
            "INSERT INTO session_aliases
             (alias_kind, alias_value, session_id, provider_id, source_id, created_at_ms)
             VALUES ('provider_session_id', 'alias-native-1', 'canonical-1', 'claude',
                     'source-canonical-1', 20)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_local_state
             (session_id, display_title, updated_at_ms)
             VALUES ('canonical-1', 'Local title', 30)",
            [],
        )
        .unwrap();

        let snapshots = SnapshotStore::new(&conn);
        for session_id in ["native-1", "canonical-1", "alias-native-1"] {
            let identity = snapshots
                .find_session_identity("claude", session_id)
                .unwrap()
                .expect("projected identity");
            assert_eq!(identity.canonical_session_id, "canonical-1");
            assert_eq!(identity.provider_session_id.as_deref(), Some("native-1"));
            assert_eq!(identity.display_title.as_deref(), Some("Local title"));
            assert_eq!(identity.workspace_dir.as_deref(), Some("/tmp/project"));
            assert_eq!(identity.last_active_at_ms, Some(20));
        }
        assert!(snapshots
            .find_session_identity("codex", "native-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn lists_paginated_workspaces_without_loading_session_details() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot_with_source_path(
            &conn,
            "one",
            "claude",
            "one",
            "/tmp/alpha/",
            "/tmp/one.jsonl",
            10,
        );
        insert_projected_snapshot_with_source_path(
            &conn,
            "two",
            "codex",
            "two",
            "/tmp/alpha",
            "/tmp/two.jsonl",
            20,
        );
        insert_projected_snapshot_with_source_path(
            &conn,
            "three",
            "claude",
            "three",
            "/tmp/beta",
            "/tmp/three.jsonl",
            30,
        );
        conn.execute(
            "UPDATE session_snapshots SET last_active_at_ms = 30 WHERE session_id = 'three'",
            [],
        )
        .unwrap();

        let store = SnapshotStore::new(&conn);
        let first = store.list_workspaces_with_sessions(None, 1, 1).unwrap();
        assert_eq!(first.total_count, 2);
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].path, "/tmp/beta");

        let second = store.list_workspaces_with_sessions(None, 2, 1).unwrap();
        assert_eq!(second.items[0].path, "/tmp/alpha");
        assert_eq!(second.items[0].session_count, 2);

        let searched = store
            .list_workspaces_with_sessions(Some("ALPHA"), 1, 5)
            .unwrap();
        assert_eq!(searched.total_count, 1);
        assert_eq!(searched.items[0].path, "/tmp/alpha");
    }

    #[test]
    fn lists_projected_identities_without_reading_provider_sources() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let file = NamedTempFile::new().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        insert_projected_snapshot_with_source_path(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            &source_path,
            20,
        );
        insert_projected_snapshot(&conn, "canonical-2", "codex", "native-2", "/tmp/other", 10);
        drop(file);

        let identities = SnapshotStore::new(&conn)
            .list_provider_session_identities("claude")
            .unwrap();

        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].canonical_session_id, "canonical-1");
        assert_eq!(
            identities[0].source_path.as_deref(),
            Some(source_path.as_str())
        );
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_fresh_source_not_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let fingerprint = required_test_source_fingerprint(file.path().to_str().unwrap());
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            file.path().to_str().unwrap(),
            &fingerprint,
            true,
        );

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness(|_, source_path| {
                test_source_fingerprint(source_path)
            })
            .unwrap();

        assert_eq!(
            report,
            SnapshotStaleScanReport {
                checked_sources: 1,
                fresh_snapshots: 1,
                stale_snapshots: 0,
                missing_sources: 0,
                unknown_sources: 0,
            }
        );
        assert!(!snapshot_stale(&conn, "canonical-1"));
    }

    #[test]
    fn session_source_freshness_requires_exact_identity_path_and_fingerprint() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        let fingerprint = required_test_source_fingerprint(file.path().to_str().unwrap());
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            &source_path,
            &fingerprint,
            false,
        );
        let store = SnapshotStore::new(&conn);

        assert!(store
            .session_source_is_fresh("claude", "native-1", &source_path, &fingerprint)
            .unwrap());
        assert!(!store
            .session_source_is_fresh("claude", "native-2", &source_path, &fingerprint)
            .unwrap());
        assert!(!store
            .session_source_is_fresh("codex", "native-1", &source_path, &fingerprint)
            .unwrap());

        writeln!(file, "two").unwrap();
        file.flush().unwrap();
        assert!(!store
            .session_source_is_fresh(
                "claude",
                "native-1",
                &source_path,
                &required_test_source_fingerprint(&source_path),
            )
            .unwrap());
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_modified_source_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let fingerprint = required_test_source_fingerprint(file.path().to_str().unwrap());
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            file.path().to_str().unwrap(),
            &fingerprint,
            false,
        );
        writeln!(file, "two").unwrap();
        file.flush().unwrap();

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness(|_, source_path| {
                test_source_fingerprint(source_path)
            })
            .unwrap();

        assert_eq!(report.checked_sources, 1);
        assert_eq!(report.fresh_snapshots, 0);
        assert_eq!(report.stale_snapshots, 1);
        assert_eq!(report.missing_sources, 0);
        assert!(snapshot_stale(&conn, "canonical-1"));
        let stored_fingerprint: String = conn
            .query_row(
                "SELECT source_fingerprint FROM session_snapshots WHERE session_id = 'canonical-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_fingerprint, fingerprint);
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_missing_source_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        let fingerprint = required_test_source_fingerprint(file.path().to_str().unwrap());
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            &source_path,
            &fingerprint,
            false,
        );
        drop(file);

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness(|_, source_path| {
                test_source_fingerprint(source_path)
            })
            .unwrap();

        assert_eq!(report.checked_sources, 0);
        assert_eq!(report.fresh_snapshots, 0);
        assert_eq!(report.stale_snapshots, 1);
        assert_eq!(report.missing_sources, 1);
        assert_eq!(report.unknown_sources, 0);
        assert!(snapshot_stale(&conn, "canonical-1"));
    }

    #[test]
    fn list_stale_snapshot_sources_filters_provider() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/claude.jsonl",
            "fingerprint-1",
            true,
        );
        insert_projected_snapshot_source(
            &conn,
            "canonical-2",
            "codex",
            "native-2",
            "/tmp/codex.jsonl",
            "fingerprint-2",
            true,
        );
        insert_projected_snapshot_source(
            &conn,
            "canonical-3",
            "claude",
            "native-3",
            "/tmp/fresh.jsonl",
            "fingerprint-3",
            false,
        );

        let rows = SnapshotStore::new(&conn)
            .list_stale_snapshot_sources(Some("claude"))
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_session_id, "canonical-1");
        assert_eq!(rows[0].provider_id, "claude");
        assert_eq!(rows[0].provider_session_id.as_deref(), Some("native-1"));
        assert_eq!(rows[0].source_path.as_deref(), Some("/tmp/claude.jsonl"));
    }

    fn insert_projected_snapshot(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        file_size_bytes: i64,
    ) {
        insert_projected_snapshot_with_source_path(
            conn,
            session_id,
            provider_id,
            provider_session_id,
            workspace_dir,
            "/tmp/source.jsonl",
            file_size_bytes,
        );
    }

    fn insert_projected_snapshot_with_source_path(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        source_path: &str,
        file_size_bytes: i64,
    ) {
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir, file_size_bytes,
              first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 10, 10)",
            params![
                format!("source-{session_id}"),
                provider_id,
                provider_session_id,
                source_path,
                workspace_dir,
                file_size_bytes,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Native title', 'completed', 10, 20, 2, 1)",
            params![
                session_id,
                provider_id,
                provider_session_id,
                format!("source-{session_id}"),
                workspace_dir,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, workspace_dir, status, last_active_at_ms,
              event_count, turn_count, flags_json, projection_version, stale, updated_at_ms)
             VALUES (?1, ?2, 'Native title', ?3, 'completed', 20, 2, 1,
              '{\"hidden\":false,\"pinned\":false}', 1, 0, 20)",
            params![session_id, provider_id, workspace_dir],
        )
        .unwrap();
    }

    fn insert_projected_snapshot_source(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        source_path: &str,
        source_fingerprint: &str,
        stale: bool,
    ) {
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, 10, 10)",
            params![
                format!("source-{session_id}"),
                provider_id,
                provider_session_id,
                source_path,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, ?2, ?3, ?4, 'Native title', 'completed', 10, 20, 2, 1)",
            params![
                session_id,
                provider_id,
                provider_session_id,
                format!("source-{session_id}"),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, status, last_active_at_ms, event_count, turn_count,
              flags_json, projection_version, source_fingerprint, stale, updated_at_ms)
             VALUES (?1, ?2, 'Native title', 'completed', 20, 2, 1, '{}', 1, ?3, ?4, 20)",
            params![
                session_id,
                provider_id,
                source_fingerprint,
                if stale { 1 } else { 0 }
            ],
        )
        .unwrap();
    }

    fn snapshot_stale(conn: &Connection, session_id: &str) -> bool {
        let stale: i64 = conn
            .query_row(
                "SELECT stale FROM session_snapshots WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        stale != 0
    }
}
