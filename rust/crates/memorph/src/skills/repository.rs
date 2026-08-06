use anyhow::{Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::BTreeSet;

const ALL_SKILL_AGENTS: [&str; 5] = ["claude", "codex", "gemini", "opencode", "hermes"];

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
    pub trigger_terms_json: String,
    pub section_index_json: String,
    pub file_manifest_json: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallationRecord {
    pub id: String,
    pub skill_id: String,
    pub used_by: String,
    pub scope_kind: String,
    pub workspace_dir: Option<String>,
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
    agent_id: Option<&str>,
    source_path: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO skill_scan_state
         (state_key, state_kind, agent_id, source_path, last_started_at_ms,
          scan_generation, completeness_status, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 'partial', ?5)
         ON CONFLICT(state_key) DO UPDATE SET
          agent_id = excluded.agent_id,
          source_path = excluded.source_path,
          last_started_at_ms = excluded.last_started_at_ms,
          scan_generation = skill_scan_state.scan_generation + 1,
          completeness_status = 'partial', error_text = NULL,
          updated_at_ms = excluded.updated_at_ms",
        params![state_key, state_kind, agent_id, source_path, now_ms],
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

pub fn session_source_scan_is_current(
    conn: &Connection,
    state_key: &str,
    fingerprint: &str,
    cursor: Option<&str>,
    catalog_generation: &str,
) -> Result<bool> {
    let state = conn
        .query_row(
            "SELECT completeness_status, source_fingerprint, source_cursor, details_json
         FROM skill_scan_state
         WHERE state_key = ?1 AND state_kind = 'session-source'",
            [state_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .context("Failed to read session source scan state")?;
    let Some((status, stored_fingerprint, stored_cursor, details)) = state else {
        return Ok(false);
    };
    let stored_generation = serde_json::from_str::<serde_json::Value>(&details)
        .ok()
        .and_then(|value| {
            value
                .get("catalog_generation")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        });
    Ok(status == "complete"
        && stored_fingerprint.as_deref() == Some(fingerprint)
        && stored_cursor.as_deref() == cursor
        && stored_generation.as_deref() == Some(catalog_generation))
}

pub fn set_scan_catalog_generation(
    conn: &Connection,
    state_key: &str,
    catalog_generation: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE skill_scan_state SET details_json = json_set(details_json, '$.catalog_generation', ?2) WHERE state_key = ?1",
        params![state_key, catalog_generation],
    )?;
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
    agent_id: &str,
    scope_kind: &str,
    workspace_dir: Option<&str>,
    root_path: &str,
    fingerprint: &str,
    catalog: &[CatalogRecord],
    installations: &[InstallationRecord],
    full: bool,
    now_ms: i64,
) -> Result<bool> {
    let state_key = format!("skill-root:{agent_id}:{root_path}");
    begin_scan(
        conn,
        &state_key,
        "skill-root",
        Some(agent_id),
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
        return Ok(false);
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
         WHERE used_by = ?1 AND scope_kind = ?2 AND workspace_dir IS ?3 AND status = 'active'
           AND id NOT IN (SELECT value FROM json_each(?4))",
        params![
            if agent_id == "agents-shared" {
                "all"
            } else {
                agent_id
            },
            scope_kind,
            workspace_dir,
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
        .context("Failed to commit skill root transaction")?;
    Ok(true)
}

/// Hard-delete one catalog row and every row that references it, by the
/// catalog row's hash id.
///
/// Used by the explicit "delete skill" endpoint, which is per-list-row: the
/// caller passes the specific `skill_catalog.id` shown in the UI, so only that
/// one skill copy is removed (not every same-named copy). Unlike a scan — which
/// marks a removed skill `missing_since_ms` and leaves the row behind as a
/// ghost — this removes the catalog row, its installations, and all derived
/// stats (coverage, usage, invocations) in FK-safe order.
pub fn delete_skill(conn: &mut Connection, catalog_id: &str) -> Result<()> {
    let tx = conn
        .transaction()
        .context("Failed to start skill deletion transaction")?;
    // Children before parents. Active tables referencing skill_catalog(id):
    // skill_coverage_observations, skill_usage_daily, skill_invocations,
    // skill_installations — then the catalog row itself.
    tx.execute(
        "DELETE FROM skill_coverage_observations WHERE skill_id = ?1",
        params![catalog_id],
    )
    .context("Failed to delete skill coverage observations")?;
    tx.execute(
        "DELETE FROM skill_usage_daily WHERE skill_id = ?1",
        params![catalog_id],
    )
    .context("Failed to delete skill usage")?;
    tx.execute(
        "DELETE FROM skill_invocations WHERE skill_id = ?1",
        params![catalog_id],
    )
    .context("Failed to delete skill invocations")?;
    tx.execute(
        "DELETE FROM skill_installations WHERE skill_id = ?1",
        params![catalog_id],
    )
    .context("Failed to delete skill installations")?;
    tx.execute("DELETE FROM skill_catalog WHERE id = ?1", params![catalog_id])
        .context("Failed to delete skill catalog row")?;
    tx.commit()
        .context("Failed to commit skill deletion")?;
    Ok(())
}

fn upsert_catalog(tx: &Transaction<'_>, skill: &CatalogRecord, now_ms: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO skill_catalog
         (id, canonical_name, normalized_name, description, version, author, entry_content_hash,
          bundle_content_hash, metadata_json, trigger_terms_json, section_index_json, file_manifest_json, file_count, total_bytes, first_seen_at_ms,
          tags_json, last_scanned_at_ms, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16, ?16, ?16)
         ON CONFLICT(id) DO UPDATE SET canonical_name = excluded.canonical_name,
          normalized_name = excluded.normalized_name, description = excluded.description,
          version = excluded.version, author = excluded.author,
          entry_content_hash = excluded.entry_content_hash,
          bundle_content_hash = excluded.bundle_content_hash,
          tags_json = excluded.tags_json,
          metadata_json = excluded.metadata_json, trigger_terms_json = excluded.trigger_terms_json,
          section_index_json = excluded.section_index_json, file_manifest_json = excluded.file_manifest_json,
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
            skill.trigger_terms_json,
            skill.section_index_json,
            skill.file_manifest_json,
            skill.file_count as i64,
            skill.total_bytes as i64,
            serde_json::to_string(&skill.tags)?,
            now_ms
        ],
    )?;
    Ok(())
}

fn upsert_installation(tx: &Transaction<'_>, item: &InstallationRecord, now_ms: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO skill_installations
         (id, skill_id, used_by, scope_kind, workspace_dir, install_path, canonical_install_path,
          install_kind, symlink_target, managed_marker_present, link_status, status, bundle_content_hash,
          discovered_at_ms, last_verified_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, ?13)
         ON CONFLICT(used_by, install_path) DO UPDATE SET
          skill_id = excluded.skill_id, scope_kind = excluded.scope_kind,
          workspace_dir = excluded.workspace_dir, install_path = excluded.install_path,
          install_kind = excluded.install_kind, symlink_target = excluded.symlink_target,
          managed_marker_present = excluded.managed_marker_present,
          link_status = excluded.link_status, bundle_content_hash = excluded.bundle_content_hash, status = 'active', removed_at_ms = NULL,
          error_text = NULL, last_verified_at_ms = excluded.last_verified_at_ms",
        params![
            item.id,
            item.skill_id,
            item.used_by,
            item.scope_kind,
            item.workspace_dir,
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
    pub used_by: Option<String>,
    pub scope: Option<String>,
    pub sort: Option<String>,
    pub descending: bool,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CatalogInstallation {
    pub used_by: String,
    pub scope_kind: String,
    pub workspace_dir: Option<String>,
    pub install_path: String,
    pub install_kind: String,
    pub symlink_target: Option<String>,
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
    pub used_by: Vec<String>,
    pub tags: Vec<String>,
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
    pub used_by: Vec<String>,
    pub completeness: CatalogCompleteness,
    /// Hint that the caller should queue a background scan: true when the
    /// catalog looks unpopulated (no items AND the skill-root scan state is
    /// unknown). Frontends use this to fire-and-forget a lightweight scan
    /// without blocking the list response.
    #[serde(default)]
    pub needs_scan: bool,
}

pub fn list_catalog_default(query: &CatalogQuery) -> Result<CatalogPage> {
    let store = crate::storage::local_store::LocalSqliteStore::open_default()?;
    list_catalog(store.connection(), query)
}

pub fn list_catalog_path(path: &std::path::Path, query: &CatalogQuery) -> Result<CatalogPage> {
    let store = crate::storage::local_store::LocalSqliteStore::open(path)?;
    list_catalog(store.connection(), query)
}

pub fn get_catalog_item_default(skill_id: &str) -> Result<Option<CatalogItem>> {
    let store = crate::storage::local_store::LocalSqliteStore::open_default()?;
    get_catalog_item(store.connection(), skill_id)
}

pub fn get_catalog_item_path(
    path: &std::path::Path,
    skill_id: &str,
) -> Result<Option<CatalogItem>> {
    let store = crate::storage::local_store::LocalSqliteStore::open(path)?;
    get_catalog_item(store.connection(), skill_id)
}

pub fn get_catalog_item(conn: &Connection, skill_id: &str) -> Result<Option<CatalogItem>> {
    let mut item = conn
        .query_row(
            "SELECT c.id, c.normalized_name, c.canonical_name, c.description, c.version, c.author,
                    c.bundle_content_hash, c.file_count, c.total_bytes,
                    c.missing_since_ms IS NOT NULL, c.updated_at_ms, c.tags_json
             FROM skill_catalog c WHERE c.id = ?1",
            [skill_id],
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
                    tags: serde_json::from_str::<Vec<String>>(&row.get::<_, String>(11)?)
                        .unwrap_or_default(),
                    used_by: Vec::new(),
                })
            },
        )
        .optional()?;
    if let Some(item) = item.as_mut() {
        populate_catalog_item_details(conn, item)?;
    }
    Ok(item)
}

fn populate_catalog_item_details(conn: &Connection, item: &mut CatalogItem) -> Result<()> {
    let mut installation_statement = conn.prepare(
        "SELECT used_by, scope_kind, workspace_dir, install_path, install_kind, symlink_target, link_status, status
         FROM skill_installations WHERE skill_id = ?1 ORDER BY used_by, canonical_install_path",
    )?;
    item.installations = installation_statement
        .query_map([&item.id], |row| {
            Ok(CatalogInstallation {
                used_by: row.get(0)?,
                scope_kind: row.get(1)?,
                workspace_dir: row.get(2)?,
                install_path: row.get(3)?,
                install_kind: row.get(4)?,
                symlink_target: row.get(5)?,
                link_status: row.get(6)?,
                status: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut used_by = BTreeSet::new();
    for installation in item
        .installations
        .iter()
        .filter(|installation| installation.status == "active")
    {
        if installation.used_by == "all" {
            used_by.extend(ALL_SKILL_AGENTS.map(str::to_string));
        } else {
            used_by.insert(installation.used_by.clone());
        }
    }
    item.used_by = used_by.into_iter().collect();
    if item.missing && !item.tags.iter().any(|tag| tag == "missing") {
        item.tags.push("missing".into());
        item.tags.sort();
    }
    Ok(())
}

pub fn list_catalog(conn: &Connection, query: &CatalogQuery) -> Result<CatalogPage> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let search = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let used_by = query.used_by.as_deref().filter(|value| !value.is_empty());
    let scope = query
        .scope
        .as_deref()
        .filter(|value| matches!(*value, "global" | "project"));
    let filters = "(?1 IS NULL OR lower(c.canonical_name) LIKE '%' || lower(?1) || '%'
                   OR lower(COALESCE(c.description, '')) LIKE '%' || lower(?1) || '%'
                   OR EXISTS (SELECT 1 FROM skill_installations si WHERE si.skill_id = c.id
                              AND lower(si.install_path) LIKE '%' || lower(?1) || '%'))
                  AND (?2 IS NULL OR EXISTS (SELECT 1 FROM skill_installations si
                       WHERE si.skill_id = c.id AND (si.used_by = ?2 OR si.used_by = 'all') AND si.status = 'active'))
                  AND (?3 IS NULL OR EXISTS (SELECT 1 FROM skill_installations si
                       WHERE si.skill_id = c.id AND si.scope_kind = ?3 AND si.status = 'active'))";
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM skill_catalog c WHERE {filters}"),
        params![search, used_by, scope],
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
                c.missing_since_ms IS NOT NULL, c.updated_at_ms, c.tags_json
         FROM skill_catalog c WHERE {filters}
         ORDER BY {order} {direction}, c.id ASC LIMIT ?4 OFFSET ?5"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(
        params![
            search,
            used_by,
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
                tags: serde_json::from_str::<Vec<String>>(&row.get::<_, String>(11)?)
                    .unwrap_or_default(),
                used_by: Vec::new(),
            })
        },
    )?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for item in &mut items {
        populate_catalog_item_details(conn, item)?;
    }
    let raw_used_by = conn.prepare(
        "SELECT DISTINCT used_by FROM skill_installations WHERE status = 'active' ORDER BY used_by",
    )?.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut used_by = BTreeSet::new();
    for value in raw_used_by {
        if value == "all" {
            used_by.extend(ALL_SKILL_AGENTS.map(str::to_string));
        } else {
            used_by.insert(value);
        }
    }
    let completeness = conn.query_row(
        "SELECT CASE
             WHEN COUNT(*) = 0 THEN 'unknown'
             WHEN SUM(CASE WHEN completeness_status = 'error' THEN 1 ELSE 0 END) > 0 THEN 'error'
             WHEN SUM(CASE WHEN completeness_status = 'partial' THEN 1 ELSE 0 END) > 0 THEN 'partial'
             ELSE 'complete'
         END,
         MAX(updated_at_ms)
         FROM skill_scan_state WHERE state_kind = 'skill-root'",
        [],
        |row| {
            Ok(CatalogCompleteness {
                status: row.get(0)?,
                updated_at_ms: row.get(1)?,
            })
        },
    )?;
    let needs_scan = total == 0 && completeness.status == "unknown";
    Ok(CatalogPage {
        items,
        page,
        page_size,
        total: total as usize,
        used_by: used_by.into_iter().collect(),
        completeness,
        needs_scan,
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
                "INSERT INTO skill_installations (id, skill_id, used_by, scope_kind, install_path,
                 canonical_install_path, install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
                 VALUES (?1, ?2, ?3, 'global', ?4, ?4, 'directory', ?2, 1, 1)",
                params![format!("install-{index}"), format!("skill-{index}"), if index == 2 { "claude" } else { "codex" }, format!("/tmp/skill-{index}")],
            ).unwrap();
        }
        let page = list_catalog(
            store.connection(),
            &CatalogQuery {
                used_by: Some("codex".into()),
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
        assert_eq!(page.used_by, vec!["claude", "codex"]);
    }

    #[test]
    fn get_catalog_item_loads_installations_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        store.connection().execute(
            "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
             bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
             created_at_ms, updated_at_ms) VALUES ('skill:sha256:abc', 'Writer', 'writer', 'entry', 'bundle', 1, 100, 1, 1, 1, 1)",
            [],
        ).unwrap();
        store.connection().execute(
            "INSERT INTO skill_installations (id, skill_id, used_by, scope_kind, install_path,
             canonical_install_path, install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
             VALUES ('install-1', 'skill:sha256:abc', 'codex', 'global', '/tmp/writer', '/tmp/writer', 'directory', 'bundle', 1, 1)",
            [],
        ).unwrap();

        let item = get_catalog_item(store.connection(), "skill:sha256:abc")
            .unwrap()
            .expect("catalog item");
        assert_eq!(item.source_id, "writer");
        assert_eq!(item.installations.len(), 1);
        assert_eq!(item.used_by, vec!["codex"]);
    }

    #[test]
    fn catalog_paginates_one_thousand_skills() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        let tx = store.connection_mut().transaction().unwrap();
        for index in 0..1_000 {
            tx.execute(
                "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
                 bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
                 created_at_ms, updated_at_ms) VALUES (?1, ?2, ?1, 'entry', ?1, 1, 100, 1, 1, 1, 1)",
                params![format!("skill-{index:04}"), format!("Skill {index:04}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();

        let page = list_catalog(
            store.connection(),
            &CatalogQuery {
                page: 20,
                page_size: 50,
                ..CatalogQuery::default()
            },
        )
        .unwrap();

        assert_eq!(page.total, 1_000);
        assert_eq!(page.items.len(), 50);
        assert_eq!(page.items[0].name, "Skill 0950");
        assert_eq!(page.items[49].name, "Skill 0999");
    }

    #[test]
    fn shared_installations_are_used_by_all_agents() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        store.connection().execute(
            "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
             bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
             created_at_ms, updated_at_ms) VALUES ('skill', 'Skill', 'skill', 'entry', 'bundle', 1, 1, 1, 1, 1, 1)",
            [],
        ).unwrap();
        store.connection().execute(
            "INSERT INTO skill_installations (id, skill_id, used_by, scope_kind, install_path,
             canonical_install_path, install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
             VALUES ('installation', 'skill', 'all', 'global', '/tmp/skill', '/tmp/skill', 'directory', 'bundle', 1, 1)",
            [],
        ).unwrap();

        let page = list_catalog(
            store.connection(),
            &CatalogQuery {
                used_by: Some("codex".into()),
                page: 1,
                page_size: 50,
                ..CatalogQuery::default()
            },
        )
        .unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].installations[0].used_by, "all");
        assert_eq!(
            page.items[0].used_by,
            vec!["claude", "codex", "gemini", "hermes", "opencode"]
        );
        assert_eq!(page.used_by, page.items[0].used_by);
    }

    #[test]
    fn catalog_completeness_aggregates_skill_root_scan_states() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        let page = list_catalog(store.connection(), &CatalogQuery::default()).unwrap();
        assert_eq!(page.completeness.status, "unknown");
        assert_eq!(page.completeness.updated_at_ms, None);
        assert!(page.needs_scan);

        for (state_key, state_kind, status, updated_at_ms) in [
            ("root-complete", "skill-root", "complete", 10),
            ("analysis-error", "aggregate", "error", 20),
            ("root-partial", "skill-root", "partial", 30),
        ] {
            store
                .connection()
                .execute(
                    "INSERT INTO skill_scan_state
                 (state_key, state_kind, completeness_status, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                    params![state_key, state_kind, status, updated_at_ms],
                )
                .unwrap();
        }

        let page = list_catalog(store.connection(), &CatalogQuery::default()).unwrap();
        assert_eq!(page.completeness.status, "partial");
        assert_eq!(page.completeness.updated_at_ms, Some(30));
        assert!(!page.needs_scan);

        store
            .connection()
            .execute(
                "INSERT INTO skill_scan_state
             (state_key, state_kind, completeness_status, updated_at_ms)
             VALUES ('root-error', 'skill-root', 'error', 25)",
                [],
            )
            .unwrap();
        let page = list_catalog(store.connection(), &CatalogQuery::default()).unwrap();
        assert_eq!(page.completeness.status, "error");
        assert_eq!(page.completeness.updated_at_ms, Some(30));
    }

    #[test]
    fn delete_skill_removes_catalog_installations_and_derived_stats() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        // One catalog row keyed by its hash id, plus an installation and usage.
        store.connection().execute(
            "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
             bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
             created_at_ms, updated_at_ms) VALUES ('skill:sha256:aaa', 'Writer', 'writer', 'entry', 'bundle', 1, 100, 1, 1, 1, 1)",
            [],
        )
        .unwrap();
        store.connection().execute(
            "INSERT INTO skill_installations (id, skill_id, used_by, scope_kind, install_path,
             canonical_install_path, install_kind, bundle_content_hash, discovered_at_ms, last_verified_at_ms)
             VALUES ('install-1', 'skill:sha256:aaa', 'codex', 'global', '/tmp/writer', '/tmp/writer', 'directory', 'bundle', 1, 1)",
            [],
        )
        .unwrap();
        store.connection().execute(
            "INSERT INTO skill_usage_daily (usage_date, skill_id, provider_id, workspace_key, invocation_count, session_count, updated_at_ms)
             VALUES ('2026-08-05', 'skill:sha256:aaa', 'codex', '', 3, 1, 1)",
            [],
        )
        .unwrap();
        // An unrelated same-named row must survive a per-id delete.
        store.connection().execute(
            "INSERT INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash,
             bundle_content_hash, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms,
             created_at_ms, updated_at_ms) VALUES ('skill:sha256:bbb', 'Writer', 'writer', 'entry2', 'bundle2', 1, 100, 1, 1, 1, 1)",
            [],
        )
        .unwrap();

        // Delete by catalog id — exactly what the per-row UI sends.
        delete_skill(store.connection_mut(), "skill:sha256:aaa").unwrap();

        let gone: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_catalog WHERE id = 'skill:sha256:aaa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let installations: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_installations WHERE skill_id = 'skill:sha256:aaa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let usage: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_usage_daily WHERE skill_id = 'skill:sha256:aaa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // The sibling same-named row is untouched — per-row, not per-name.
        let sibling: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_catalog WHERE id = 'skill:sha256:bbb'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
        assert_eq!(installations, 0);
        assert_eq!(usage, 0);
        assert_eq!(sibling, 1);
    }
}
