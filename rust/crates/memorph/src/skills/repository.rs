use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRecord {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub entry_hash: String,
    pub bundle_hash: String,
    pub metadata_json: String,
    pub file_manifest_json: String,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallationRecord {
    pub id: String,
    pub skill_id: String,
    pub provider_id: String,
    pub install_path: String,
    pub canonical_path: String,
    pub install_kind: String,
    pub symlink_target: Option<String>,
    pub managed_marker_present: bool,
    pub link_status: String,
    pub bundle_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSourceRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub provider_id: String,
    pub source_path: String,
    pub workspace_dir: Option<String>,
    pub source_cursor: Option<String>,
    pub fingerprint: String,
    pub earliest_at_ms: Option<i64>,
    pub latest_at_ms: Option<i64>,
}

pub fn fail_scan(conn: &Connection, state_key: &str, error: &str, now_ms: i64) -> Result<()> {
    conn.execute(
        "UPDATE skill_scan_state SET completeness_status = 'error', error_text = ?2,
         updated_at_ms = ?3 WHERE state_key = ?1",
        params![state_key, error, now_ms],
    )?;
    Ok(())
}

pub fn begin_scan(
    conn: &Connection,
    state_key: &str,
    state_kind: &str,
    provider_id: Option<&str>,
    source_path: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO skill_scan_state
         (state_key, state_kind, provider_id, source_path, last_started_at_ms,
          scan_generation, completeness_status, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 'partial', ?5)
         ON CONFLICT(state_key) DO UPDATE SET
          provider_id = excluded.provider_id,
          source_path = excluded.source_path,
          last_started_at_ms = excluded.last_started_at_ms,
          scan_generation = skill_scan_state.scan_generation + 1,
          completeness_status = 'partial', error_text = NULL,
          updated_at_ms = excluded.updated_at_ms",
        params![state_key, state_kind, provider_id, source_path, now_ms],
    )
    .with_context(|| format!("Failed to begin skill scan state {state_key}"))?;
    Ok(())
}

pub fn complete_scan(
    conn: &Connection,
    state_key: &str,
    fingerprint: Option<&str>,
    cursor: Option<&str>,
    items_seen: usize,
    full: bool,
    completeness: &str,
    earliest_at_ms: Option<i64>,
    latest_at_ms: Option<i64>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE skill_scan_state SET
          source_fingerprint = ?2, source_cursor = ?3, last_completed_at_ms = ?4,
          last_full_scan_at_ms = CASE WHEN ?5 THEN ?4 ELSE last_full_scan_at_ms END,
          completeness_status = ?6, earliest_indexed_at_ms = ?7,
          latest_indexed_at_ms = ?8, items_seen = ?9, error_text = NULL,
          updated_at_ms = ?4
         WHERE state_key = ?1",
        params![
            state_key,
            fingerprint,
            cursor,
            now_ms,
            full,
            completeness,
            earliest_at_ms,
            latest_at_ms,
            items_seen as i64
        ],
    )
    .with_context(|| format!("Failed to complete skill scan state {state_key}"))?;
    Ok(())
}

pub fn scan_fingerprint(conn: &Connection, state_key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT source_fingerprint FROM skill_scan_state WHERE state_key = ?1",
        [state_key],
        |row| row.get(0),
    )
    .optional()
    .context("Failed to read skill scan fingerprint")
    .map(Option::flatten)
}

pub fn persist_root(
    conn: &mut Connection,
    provider_id: &str,
    root_path: &str,
    fingerprint: &str,
    catalog: &[CatalogRecord],
    installations: &[InstallationRecord],
    full: bool,
    now_ms: i64,
) -> Result<()> {
    let state_key = format!("skill-root:{provider_id}:{root_path}");
    begin_scan(
        conn,
        &state_key,
        "skill-root",
        Some(provider_id),
        Some(root_path),
        now_ms,
    )?;
    if !full && scan_fingerprint(conn, &state_key)?.as_deref() == Some(fingerprint) {
        complete_scan(
            conn,
            &state_key,
            Some(fingerprint),
            None,
            installations.len(),
            false,
            "complete",
            None,
            None,
            now_ms,
        )?;
        return Ok(());
    }

    let tx = conn
        .transaction()
        .context("Failed to start skill root transaction")?;
    for skill in catalog {
        upsert_catalog(&tx, skill, now_ms)?;
    }
    for installation in installations {
        upsert_installation(&tx, installation, now_ms)?;
    }
    tx.execute(
        "UPDATE skill_installations SET status = 'missing', error_text = NULL
         WHERE provider_id = ?1 AND status = 'active'
           AND id NOT IN (SELECT value FROM json_each(?2))",
        params![
            provider_id,
            serde_json::to_string(
                &installations
                    .iter()
                    .map(|item| &item.id)
                    .collect::<Vec<_>>()
            )?
        ],
    )
    .context("Failed to mark missing skill installations")?;
    tx.execute(
        "UPDATE skill_catalog SET missing_since_ms = COALESCE(missing_since_ms, ?1), updated_at_ms = ?1
         WHERE id NOT IN (SELECT DISTINCT skill_id FROM skill_installations WHERE status = 'active')",
        [now_ms],
    )?;
    tx.execute(
        "UPDATE skill_catalog SET missing_since_ms = NULL
         WHERE id IN (SELECT DISTINCT skill_id FROM skill_installations WHERE status = 'active')",
        [],
    )?;
    complete_scan(
        &tx,
        &state_key,
        Some(fingerprint),
        None,
        installations.len(),
        full,
        "complete",
        None,
        None,
        now_ms,
    )?;
    tx.commit()
        .context("Failed to commit skill root transaction")
}

fn upsert_catalog(tx: &Transaction<'_>, skill: &CatalogRecord, now_ms: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO skill_catalog
         (id, canonical_name, normalized_name, description, version, author, entry_content_hash,
          bundle_content_hash, metadata_json, file_manifest_json, file_count, total_bytes, first_seen_at_ms,
          last_scanned_at_ms, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?13, ?13)
         ON CONFLICT(id) DO UPDATE SET canonical_name = excluded.canonical_name,
          normalized_name = excluded.normalized_name, description = excluded.description,
          version = excluded.version, author = excluded.author,
          entry_content_hash = excluded.entry_content_hash,
          bundle_content_hash = excluded.bundle_content_hash,
          metadata_json = excluded.metadata_json, file_manifest_json = excluded.file_manifest_json,
          file_count = excluded.file_count,
          total_bytes = excluded.total_bytes, last_scanned_at_ms = excluded.last_scanned_at_ms,
          missing_since_ms = NULL, scan_error = NULL, updated_at_ms = excluded.updated_at_ms",
        params![
            skill.id,
            skill.name,
            skill.normalized_name,
            skill.description,
            skill.version,
            skill.author,
            skill.entry_hash,
            skill.bundle_hash,
            skill.metadata_json,
            skill.file_manifest_json,
            skill.file_count as i64,
            skill.total_bytes as i64,
            now_ms
        ],
    )?;
    Ok(())
}

fn upsert_installation(tx: &Transaction<'_>, item: &InstallationRecord, now_ms: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO skill_installations
         (id, skill_id, provider_id, scope_kind, install_path, canonical_install_path,
          install_kind, symlink_target, managed_marker_present, link_status, status, bundle_content_hash,
          discovered_at_ms, last_verified_at_ms)
         VALUES (?1, ?2, ?3, 'global', ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?11)
         ON CONFLICT(provider_id, canonical_install_path) DO UPDATE SET
          skill_id = excluded.skill_id, install_path = excluded.install_path,
          install_kind = excluded.install_kind, symlink_target = excluded.symlink_target,
          managed_marker_present = excluded.managed_marker_present,
          link_status = excluded.link_status, bundle_content_hash = excluded.bundle_content_hash, status = 'active', removed_at_ms = NULL,
          error_text = NULL, last_verified_at_ms = excluded.last_verified_at_ms",
        params![
            item.id,
            item.skill_id,
            item.provider_id,
            item.install_path,
            item.canonical_path,
            item.install_kind,
            item.symlink_target,
            item.managed_marker_present,
            item.link_status,
            item.bundle_hash,
            now_ms
        ],
    )?;
    Ok(())
}

pub fn session_sources(conn: &Connection) -> Result<Vec<SessionSourceRecord>> {
    let mut statement = conn.prepare(
        "SELECT src.id, s.id, s.provider_session_id, src.provider_id, src.source_path,
                COALESCE(s.workspace_dir, src.workspace_dir), src.source_cursor,
                printf('%s:%s:%s:%s', COALESCE(src.file_mtime_ms, 0), COALESCE(src.file_size_bytes, 0), COALESCE(src.content_hash, ''), COALESCE(s.id, '')),
                s.created_at_ms, s.last_active_at_ms
         FROM session_sources src
         LEFT JOIN sessions s ON s.primary_source_id = src.id
         ORDER BY src.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SessionSourceRecord {
            id: row.get(0)?,
            session_id: row.get(1)?,
            provider_session_id: row.get(2)?,
            provider_id: row.get(3)?,
            source_path: row.get(4)?,
            workspace_dir: row.get(5)?,
            source_cursor: row.get(6)?,
            fingerprint: row.get(7)?,
            earliest_at_ms: row.get(8)?,
            latest_at_ms: row.get(9)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to list session sources")
}

#[derive(Clone, Debug, Default)]
pub struct CatalogQuery {
    pub query: Option<String>,
    pub provider: Option<String>,
    pub scope: Option<String>,
    pub sort: Option<String>,
    pub descending: bool,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CatalogInstallation {
    pub provider_id: String,
    pub scope_kind: String,
    pub workspace_dir: Option<String>,
    pub install_path: String,
    pub install_kind: String,
    pub link_status: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CatalogItem {
    pub id: String,
    pub source_id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub bundle_hash: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub missing: bool,
    pub updated_at_ms: i64,
    pub installations: Vec<CatalogInstallation>,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CatalogCompleteness {
    pub status: String,
    pub updated_at_ms: Option<i64>,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CatalogPage {
    pub items: Vec<CatalogItem>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub providers: Vec<String>,
    pub completeness: CatalogCompleteness,
}

pub fn list_catalog_default(query: &CatalogQuery) -> Result<CatalogPage> {
    let store = crate::storage::local_store::LocalSqliteStore::open_default()?;
    list_catalog(store.connection(), query)
}

pub fn list_catalog_path(path: &std::path::Path, query: &CatalogQuery) -> Result<CatalogPage> {
    let store = crate::storage::local_store::LocalSqliteStore::open(path)?;
    list_catalog(store.connection(), query)
}

pub fn list_catalog(conn: &Connection, query: &CatalogQuery) -> Result<CatalogPage> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let search = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let provider = query
        .provider
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "all");
    let scope = query
        .scope
        .as_deref()
        .filter(|value| matches!(*value, "global" | "project"));
    let filters = "(?1 IS NULL OR lower(c.canonical_name) LIKE '%' || lower(?1) || '%'
                   OR lower(COALESCE(c.description, '')) LIKE '%' || lower(?1) || '%'
                   OR EXISTS (SELECT 1 FROM skill_installations si WHERE si.skill_id = c.id
                              AND lower(si.install_path) LIKE '%' || lower(?1) || '%'))
                  AND (?2 IS NULL OR EXISTS (SELECT 1 FROM skill_installations si
                       WHERE si.skill_id = c.id AND si.provider_id = ?2 AND si.status = 'active'))
                  AND (?3 IS NULL OR EXISTS (SELECT 1 FROM skill_installations si
                       WHERE si.skill_id = c.id AND si.scope_kind = ?3 AND si.status = 'active'))";
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM skill_catalog c WHERE {filters}"),
        params![search, provider, scope],
        |row| row.get(0),
    )?;
    let order = match query.sort.as_deref() {
        Some("size") => "c.total_bytes",
        Some("files") => "c.file_count",
        Some("updated") => "c.updated_at_ms",
        _ => "c.canonical_name COLLATE NOCASE",
    };
    let direction = if query.descending { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT c.id, c.normalized_name, c.canonical_name, c.description, c.version, c.author,
                c.bundle_content_hash, c.file_count, c.total_bytes,
                c.missing_since_ms IS NOT NULL, c.updated_at_ms
         FROM skill_catalog c WHERE {filters}
         ORDER BY {order} {direction}, c.id ASC LIMIT ?4 OFFSET ?5"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(
        params![
            search,
            provider,
            scope,
            page_size as i64,
            ((page - 1) * page_size) as i64
        ],
        |row| {
            Ok(CatalogItem {
                id: row.get(0)?,
                source_id: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                version: row.get(4)?,
                author: row.get(5)?,
                bundle_hash: row.get(6)?,
                file_count: row.get::<_, i64>(7)? as u64,
                total_bytes: row.get::<_, i64>(8)? as u64,
                missing: row.get(9)?,
                updated_at_ms: row.get(10)?,
                installations: Vec::new(),
            })
        },
    )?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut installation_statement = conn.prepare(
        "SELECT provider_id, scope_kind, workspace_dir, install_path, install_kind, link_status, status
         FROM skill_installations WHERE skill_id = ?1 ORDER BY provider_id, canonical_install_path",
    )?;
    for item in &mut items {
        item.installations = installation_statement
            .query_map([&item.id], |row| {
                Ok(CatalogInstallation {
                    provider_id: row.get(0)?,
                    scope_kind: row.get(1)?,
                    workspace_dir: row.get(2)?,
                    install_path: row.get(3)?,
                    install_kind: row.get(4)?,
                    link_status: row.get(5)?,
                    status: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    let providers = conn.prepare(
        "SELECT DISTINCT provider_id FROM skill_installations WHERE status = 'active' ORDER BY provider_id",
    )?.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let completeness = conn
        .query_row(
            "SELECT completeness_status, updated_at_ms FROM skill_scan_state
         WHERE state_kind = 'aggregate' ORDER BY updated_at_ms DESC LIMIT 1",
            [],
            |row| {
                Ok(CatalogCompleteness {
                    status: row.get(0)?,
                    updated_at_ms: row.get(1)?,
                })
            },
        )
        .optional()?
        .unwrap_or(CatalogCompleteness {
            status: "unknown".into(),
            updated_at_ms: None,
        });
    Ok(CatalogPage {
        items,
        page,
        page_size,
        total: total as usize,
        providers,
        completeness,
    })
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::storage::local_store::LocalSqliteStore;

    #[test]
    fn catalog_filters_sorts_and_paginates_in_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        for index in 0..3 {
            store.connection().execute(
                "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
                 bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
                 created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, 'entry', ?1, ?4, ?5, 1, 1, 1, 1)",
                params![format!("skill-{index}"), format!("Skill {index}"), format!("skill-{index}"), index + 1, (index + 1) * 100],
            ).unwrap();
            store.connection().execute(
                "INSERT INTO skill_installations (id, skill_id, provider_id, scope_kind, install_path,
                 canonical_install_path, install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
                 VALUES (?1, ?2, ?3, 'global', ?4, ?4, 'directory', ?2, 1, 1)",
                params![format!("install-{index}"), format!("skill-{index}"), if index == 2 { "claude" } else { "codex" }, format!("/tmp/skill-{index}")],
            ).unwrap();
        }
        let page = list_catalog(
            store.connection(),
            &CatalogQuery {
                provider: Some("codex".into()),
                sort: Some("size".into()),
                descending: true,
                page: 1,
                page_size: 1,
                ..CatalogQuery::default()
            },
        )
        .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].source_id, "skill-1");
        assert_eq!(page.providers, vec!["claude", "codex"]);
    }
}
