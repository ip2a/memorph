use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRecord {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub description: Option<String>,
    pub entry_hash: String,
    pub bundle_hash: String,
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
    pub provider_id: String,
    pub source_path: String,
    pub source_cursor: Option<String>,
    pub fingerprint: String,
    pub earliest_at_ms: Option<i64>,
    pub latest_at_ms: Option<i64>,
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
         (id, canonical_name, normalized_name, description, entry_content_hash,
          bundle_content_hash, file_count, total_bytes, first_seen_at_ms,
          last_scanned_at_ms, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9, ?9)
         ON CONFLICT(id) DO UPDATE SET canonical_name = excluded.canonical_name,
          normalized_name = excluded.normalized_name, description = excluded.description,
          entry_content_hash = excluded.entry_content_hash,
          bundle_content_hash = excluded.bundle_content_hash, file_count = excluded.file_count,
          total_bytes = excluded.total_bytes, last_scanned_at_ms = excluded.last_scanned_at_ms,
          missing_since_ms = NULL, scan_error = NULL, updated_at_ms = excluded.updated_at_ms",
        params![
            skill.id,
            skill.name,
            skill.normalized_name,
            skill.description,
            skill.entry_hash,
            skill.bundle_hash,
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
        "SELECT src.id, src.provider_id, src.source_path, src.source_cursor,
                printf('%s:%s:%s', COALESCE(src.file_mtime_ms, 0), COALESCE(src.file_size_bytes, 0), COALESCE(src.content_hash, '')),
                s.created_at_ms, s.last_active_at_ms
         FROM session_sources src
         LEFT JOIN sessions s ON s.primary_source_id = src.id
         ORDER BY src.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SessionSourceRecord {
            id: row.get(0)?,
            provider_id: row.get(1)?,
            source_path: row.get(2)?,
            source_cursor: row.get(3)?,
            fingerprint: row.get(4)?,
            earliest_at_ms: row.get(5)?,
            latest_at_ms: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to list session sources")
}
