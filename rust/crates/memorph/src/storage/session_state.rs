use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{btree_map::Entry, BTreeMap};

use crate::canonical::{LocalSessionState, SessionLocator, WorkspaceSessionState};
use crate::session_projection::{SessionIdentity, SessionIdentityInput};

use super::local_store;

#[derive(Debug, Clone, Default)]
pub struct SessionStateStore {
    pub sessions: BTreeMap<String, BTreeMap<String, LocalSessionState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResolvedLocalSessionState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_archive_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionLocalStateUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compressed_archive_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_override: Option<WorkspaceLocalStateUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WorkspaceLocalStateUpdate {
    pub workspace_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Option<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<Option<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Option<Vec<String>>>,
}

pub fn load_state_store() -> Result<SessionStateStore> {
    let conn = local_store::open_database()?;
    load_state_store_from_connection(&conn)
}

pub fn get_session_state<'a>(
    store: &'a SessionStateStore,
    provider_id: &str,
    session_id: &str,
) -> Option<&'a LocalSessionState> {
    store
        .sessions
        .get(provider_id)
        .and_then(|sessions| sessions.get(session_id))
}

pub fn resolve_session_state(
    store: &SessionStateStore,
    provider_id: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> ResolvedLocalSessionState {
    let Some(state) = get_session_state(store, provider_id, session_id) else {
        return ResolvedLocalSessionState::default();
    };
    resolve_local_state(state, workspace_dir)
}

pub fn set_session_state(mut state: LocalSessionState) -> Result<()> {
    state.updated_at = Utc::now();
    let mut conn = local_store::open_database()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start local session state transaction")?;
    let canonical_session_id = ensure_session(
        &tx,
        &state.locator.provider_id,
        &state.locator.session_id,
        state.updated_at.timestamp_millis(),
    )?;
    persist_state(&tx, &canonical_session_id, &state)?;
    tx.commit()
        .context("Failed to commit local session state transaction")
}

pub fn set_display_title(provider_id: &str, session_id: &str, title: &str) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(());
    }
    update_session_state(
        provider_id,
        session_id,
        &SessionLocalStateUpdate {
            display_title: Some(Some(title.to_string())),
            ..Default::default()
        },
    )
    .map(|_| ())
}

pub fn update_session_state(
    provider_id: &str,
    session_id: &str,
    update: &SessionLocalStateUpdate,
) -> Result<ResolvedLocalSessionState> {
    let mut conn = local_store::open_database()?;
    update_session_state_with_connection(&mut conn, provider_id, session_id, update)
}

pub fn remove_session(provider_id: &str, session_id: &str) -> Result<()> {
    let mut conn = local_store::open_database()?;
    remove_session_with_connection(&mut conn, provider_id, session_id)
}

fn remove_session_with_connection(
    conn: &mut Connection,
    provider_id: &str,
    session_id: &str,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start local session state removal")?;
    if let Some(canonical_session_id) = resolve_canonical_session_id(&tx, provider_id, session_id)?
    {
        tx.execute(
            "DELETE FROM workspace_session_state WHERE session_id = ?1",
            [&canonical_session_id],
        )
        .context("Failed to remove workspace session state")?;
        tx.execute(
            "DELETE FROM session_local_state WHERE session_id = ?1",
            [&canonical_session_id],
        )
        .context("Failed to remove local session state")?;
        tx.execute(
            "UPDATE sessions SET deleted_at_ms = ?2 WHERE id = ?1",
            params![canonical_session_id, Utc::now().timestamp_millis()],
        )
        .context("Failed to mark session as deleted")?;
    }
    tx.commit()
        .context("Failed to commit local session state removal")
}

pub fn set_session_state_in_store(store: &mut SessionStateStore, mut state: LocalSessionState) {
    state.updated_at = Utc::now();
    store
        .sessions
        .entry(state.locator.provider_id.clone())
        .or_default()
        .insert(state.locator.session_id.clone(), state);
}

pub fn set_display_title_in_store(
    store: &mut SessionStateStore,
    provider_id: &str,
    session_id: &str,
    title: &str,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    let state = state_entry(store, provider_id, session_id);
    state.display_title = Some(title.to_string());
    state.updated_at = Utc::now();
}

pub fn update_session_state_in_store(
    store: &mut SessionStateStore,
    provider_id: &str,
    session_id: &str,
    update: &SessionLocalStateUpdate,
) {
    let state = state_entry(store, provider_id, session_id);
    apply_update(state, update);
}

pub fn remove_session_in_store(store: &mut SessionStateStore, provider_id: &str, session_id: &str) {
    let Some(sessions) = store.sessions.get_mut(provider_id) else {
        return;
    };
    sessions.remove(session_id);
    if sessions.is_empty() {
        store.sessions.remove(provider_id);
    }
}

fn load_state_store_from_connection(conn: &Connection) -> Result<SessionStateStore> {
    let mut stmt = conn
        .prepare(
            "SELECT
                s.provider_id,
                COALESCE(
                    s.provider_session_id,
                    (
                        SELECT alias.alias_value
                        FROM session_aliases alias
                        WHERE alias.session_id = s.id
                          AND alias.alias_kind = 'provider_session_id'
                          AND (alias.provider_id = s.provider_id OR alias.provider_id IS NULL)
                        ORDER BY alias.created_at_ms ASC
                        LIMIT 1
                    ),
                    s.id
                ),
                local.display_title,
                local.archived,
                local.hidden,
                local.pinned,
                local.notes,
                local.tags_json,
                local.preferred_targets_json,
                local.compressed_archive_refs_json,
                local.updated_at_ms,
                workspace.workspace_dir,
                workspace.hidden,
                workspace.pinned,
                workspace.preferred_targets_json
             FROM session_local_state local
             JOIN sessions s ON s.id = local.session_id
             LEFT JOIN workspace_session_state workspace ON workspace.session_id = s.id
             ORDER BY s.provider_id, s.id, workspace.workspace_dir",
        )
        .context("Failed to prepare local session state query")?;
    let mut rows = stmt
        .query([])
        .context("Failed to query local session state")?;
    let mut store = SessionStateStore::default();

    while let Some(row) = rows
        .next()
        .context("Failed to decode local session state")?
    {
        let provider_id: String = row.get(0)?;
        let provider_session_id: String = row.get(1)?;
        let display_title: Option<String> = row.get(2)?;
        let archived = row.get::<_, i64>(3)? != 0;
        let hidden = row.get::<_, i64>(4)? != 0;
        let pinned = row.get::<_, i64>(5)? != 0;
        let notes: Option<String> = row.get(6)?;
        let tags_json: String = row.get(7)?;
        let preferred_targets_json: String = row.get(8)?;
        let compressed_archive_refs_json: String = row.get(9)?;
        let updated_at_ms: i64 = row.get(10)?;
        let workspace_dir: Option<String> = row.get(11)?;
        let workspace_hidden: Option<i64> = row.get(12)?;
        let workspace_pinned: Option<i64> = row.get(13)?;
        let workspace_preferred_targets_json: Option<String> = row.get(14)?;
        let sessions = store.sessions.entry(provider_id.clone()).or_default();
        let state = match sessions.entry(provider_session_id.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(LocalSessionState {
                locator: SessionLocator {
                    provider_id: provider_id.clone(),
                    session_id: provider_session_id.clone(),
                },
                display_title,
                archived,
                hidden,
                pinned,
                notes,
                tags: parse_string_list(&tags_json, "tags_json")?,
                preferred_targets: parse_string_list(
                    &preferred_targets_json,
                    "preferred_targets_json",
                )?,
                workspace_overrides: Vec::new(),
                compressed_archive_refs: parse_string_list(
                    &compressed_archive_refs_json,
                    "compressed_archive_refs_json",
                )?,
                updated_at: timestamp_from_millis(updated_at_ms)?,
            }),
        };

        if let Some(workspace_dir) = workspace_dir {
            state.workspace_overrides.push(WorkspaceSessionState {
                workspace_dir,
                hidden: workspace_hidden.map(|value| value != 0),
                pinned: workspace_pinned.map(|value| value != 0),
                preferred_targets: workspace_preferred_targets_json
                    .map(|raw| parse_string_list(&raw, "workspace preferred_targets_json"))
                    .transpose()?,
            });
        }
    }
    Ok(store)
}

fn update_session_state_with_connection(
    conn: &mut Connection,
    provider_id: &str,
    session_id: &str,
    update: &SessionLocalStateUpdate,
) -> Result<ResolvedLocalSessionState> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start local session state update")?;
    let now = Utc::now();
    let canonical_session_id =
        ensure_session(&tx, provider_id, session_id, now.timestamp_millis())?;
    let mut state =
        load_state_by_canonical_id(&tx, &canonical_session_id, provider_id, session_id)?
            .unwrap_or_else(|| empty_state(provider_id, session_id, now));
    apply_update(&mut state, update);
    persist_state(&tx, &canonical_session_id, &state)?;
    let workspace_dir = update
        .workspace_override
        .as_ref()
        .map(|workspace| workspace.workspace_dir.as_str());
    let resolved = resolve_local_state(&state, workspace_dir);
    tx.commit()
        .context("Failed to commit local session state update")?;
    Ok(resolved)
}

fn resolve_canonical_session_id(
    conn: &Connection,
    provider_id: &str,
    provider_session_id: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT s.id
         FROM sessions s
         WHERE s.provider_id = ?1
           AND (
                s.provider_session_id = ?2
                OR s.id = ?2
                OR EXISTS (
                    SELECT 1
                    FROM session_aliases alias
                    WHERE alias.session_id = s.id
                      AND alias.alias_kind = 'provider_session_id'
                      AND alias.alias_value = ?2
                      AND (alias.provider_id = ?1 OR alias.provider_id IS NULL)
                )
           )
         ORDER BY CASE WHEN s.provider_session_id = ?2 THEN 0 ELSE 1 END
         LIMIT 1",
        params![provider_id, provider_session_id],
        |row| row.get(0),
    )
    .optional()
    .context("Failed to resolve canonical session identity")
}

fn ensure_session(
    conn: &Connection,
    provider_id: &str,
    provider_session_id: &str,
    now_ms: i64,
) -> Result<String> {
    if let Some(session_id) = resolve_canonical_session_id(conn, provider_id, provider_session_id)?
    {
        return Ok(session_id);
    }
    let identity = SessionIdentity::from_source(SessionIdentityInput {
        provider_id,
        provider_session_id: Some(provider_session_id),
        source_path: None,
        workspace_dir: None,
    })?;
    conn.execute(
        "INSERT INTO sessions
         (id, provider_id, provider_session_id, status, updated_at_ms, projection_version)
         VALUES (?1, ?2, ?3, 'unknown', ?4, 0)
         ON CONFLICT(id) DO UPDATE SET
            provider_id = excluded.provider_id,
            provider_session_id = COALESCE(sessions.provider_session_id, excluded.provider_session_id),
            updated_at_ms = excluded.updated_at_ms",
        params![
            identity.canonical_session_id,
            identity.provider_id,
            identity.provider_session_id,
            now_ms,
        ],
    )
    .context("Failed to create managed session identity")?;
    Ok(identity.canonical_session_id)
}

fn load_state_by_canonical_id(
    conn: &Connection,
    canonical_session_id: &str,
    provider_id: &str,
    provider_session_id: &str,
) -> Result<Option<LocalSessionState>> {
    let base = conn
        .query_row(
            "SELECT display_title, archived, hidden, pinned, notes, tags_json,
                    preferred_targets_json, compressed_archive_refs_json, updated_at_ms
             FROM session_local_state
             WHERE session_id = ?1",
            [canonical_session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .context("Failed to load local session state")?;
    let Some((display_title, archived, hidden, pinned, notes, tags, targets, refs, updated)) = base
    else {
        return Ok(None);
    };
    let mut state = LocalSessionState {
        locator: SessionLocator {
            provider_id: provider_id.to_string(),
            session_id: provider_session_id.to_string(),
        },
        display_title,
        archived: archived != 0,
        hidden: hidden != 0,
        pinned: pinned != 0,
        notes,
        tags: parse_string_list(&tags, "tags_json")?,
        preferred_targets: parse_string_list(&targets, "preferred_targets_json")?,
        workspace_overrides: Vec::new(),
        compressed_archive_refs: parse_string_list(&refs, "compressed_archive_refs_json")?,
        updated_at: timestamp_from_millis(updated)?,
    };
    let mut stmt = conn
        .prepare(
            "SELECT workspace_dir, hidden, pinned, preferred_targets_json
             FROM workspace_session_state
             WHERE session_id = ?1
             ORDER BY workspace_dir",
        )
        .context("Failed to prepare workspace session state query")?;
    let mut rows = stmt
        .query([canonical_session_id])
        .context("Failed to query workspace session state")?;
    while let Some(row) = rows
        .next()
        .context("Failed to decode workspace session state")?
    {
        state.workspace_overrides.push(WorkspaceSessionState {
            workspace_dir: row.get(0)?,
            hidden: row.get::<_, Option<i64>>(1)?.map(|value| value != 0),
            pinned: row.get::<_, Option<i64>>(2)?.map(|value| value != 0),
            preferred_targets: row
                .get::<_, Option<String>>(3)?
                .map(|raw| parse_string_list(&raw, "workspace preferred_targets_json"))
                .transpose()?,
        });
    }
    Ok(Some(state))
}

fn persist_state(
    tx: &Transaction<'_>,
    canonical_session_id: &str,
    state: &LocalSessionState,
) -> Result<()> {
    let tags = serde_json::to_string(&state.tags)?;
    let preferred_targets = serde_json::to_string(&state.preferred_targets)?;
    let compressed_archive_refs = serde_json::to_string(&state.compressed_archive_refs)?;
    tx.execute(
        "INSERT INTO session_local_state
         (session_id, display_title, archived, hidden, pinned, notes, tags_json,
          preferred_targets_json, compressed_archive_refs_json, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(session_id) DO UPDATE SET
            display_title = excluded.display_title,
            archived = excluded.archived,
            hidden = excluded.hidden,
            pinned = excluded.pinned,
            notes = excluded.notes,
            tags_json = excluded.tags_json,
            preferred_targets_json = excluded.preferred_targets_json,
            compressed_archive_refs_json = excluded.compressed_archive_refs_json,
            updated_at_ms = excluded.updated_at_ms",
        params![
            canonical_session_id,
            state.display_title,
            state.archived,
            state.hidden,
            state.pinned,
            state.notes,
            tags,
            preferred_targets,
            compressed_archive_refs,
            state.updated_at.timestamp_millis(),
        ],
    )
    .context("Failed to persist local session state")?;
    tx.execute(
        "DELETE FROM workspace_session_state WHERE session_id = ?1",
        [canonical_session_id],
    )
    .context("Failed to replace workspace session state")?;
    for workspace in &state.workspace_overrides {
        if workspace_override_is_empty(workspace) {
            continue;
        }
        let preferred_targets = workspace
            .preferred_targets
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            "INSERT INTO workspace_session_state
             (session_id, workspace_dir, hidden, pinned, preferred_targets_json, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                canonical_session_id,
                workspace.workspace_dir,
                workspace.hidden,
                workspace.pinned,
                preferred_targets,
                state.updated_at.timestamp_millis(),
            ],
        )
        .context("Failed to persist workspace session state")?;
    }
    Ok(())
}

fn empty_state(
    provider_id: &str,
    session_id: &str,
    updated_at: chrono::DateTime<Utc>,
) -> LocalSessionState {
    LocalSessionState {
        locator: SessionLocator {
            provider_id: provider_id.to_string(),
            session_id: session_id.to_string(),
        },
        display_title: None,
        archived: false,
        hidden: false,
        pinned: false,
        notes: None,
        tags: Vec::new(),
        preferred_targets: Vec::new(),
        workspace_overrides: Vec::new(),
        compressed_archive_refs: Vec::new(),
        updated_at,
    }
}

fn state_entry<'a>(
    store: &'a mut SessionStateStore,
    provider_id: &str,
    session_id: &str,
) -> &'a mut LocalSessionState {
    store
        .sessions
        .entry(provider_id.to_string())
        .or_default()
        .entry(session_id.to_string())
        .or_insert_with(|| empty_state(provider_id, session_id, Utc::now()))
}

fn apply_update(state: &mut LocalSessionState, update: &SessionLocalStateUpdate) {
    if let Some(display_title) = &update.display_title {
        state.display_title = display_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if let Some(hidden) = update.hidden {
        state.hidden = hidden;
    }
    if let Some(pinned) = update.pinned {
        state.pinned = pinned;
    }
    if let Some(notes) = &update.notes {
        state.notes = notes
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if let Some(tags) = &update.tags {
        state.tags = normalize_tags(tags);
    }
    if let Some(preferred_targets) = &update.preferred_targets {
        state.preferred_targets = crate::config::normalize_provider_ids(preferred_targets.clone());
    }
    if let Some(refs) = &update.compressed_archive_refs {
        state.compressed_archive_refs = refs.clone();
    }
    if let Some(workspace_override) = &update.workspace_override {
        apply_workspace_override_update(state, workspace_override);
    }
    state.updated_at = Utc::now();
}

fn resolve_local_state(
    state: &LocalSessionState,
    workspace_dir: Option<&str>,
) -> ResolvedLocalSessionState {
    let mut resolved = ResolvedLocalSessionState {
        display_title: state
            .display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        archived: state.archived,
        hidden: state.hidden,
        pinned: state.pinned,
        notes: state.notes.clone(),
        tags: state.tags.clone(),
        preferred_targets: state.preferred_targets.clone(),
        compressed_archive_refs: state.compressed_archive_refs.clone(),
    };
    if let Some(workspace_dir) = workspace_dir {
        if let Some(workspace_state) = state
            .workspace_overrides
            .iter()
            .find(|entry| workspace_override_matches(&entry.workspace_dir, workspace_dir))
        {
            if let Some(hidden) = workspace_state.hidden {
                resolved.hidden = hidden;
            }
            if let Some(pinned) = workspace_state.pinned {
                resolved.pinned = pinned;
            }
            if let Some(preferred_targets) = workspace_state.preferred_targets.as_ref() {
                resolved.preferred_targets = preferred_targets.clone();
            }
        }
    }
    resolved
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || normalized.iter().any(|existing| existing == tag) {
            continue;
        }
        normalized.push(tag.to_string());
    }
    normalized
}

fn workspace_override_matches(stored_workspace: &str, requested_workspace: &str) -> bool {
    stored_workspace == requested_workspace
        || crate::provider::default_workspace_matches(
            Some(stored_workspace),
            Some(requested_workspace),
        )
}

fn apply_workspace_override_update(
    state: &mut LocalSessionState,
    update: &WorkspaceLocalStateUpdate,
) {
    let workspace_dir = update.workspace_dir.trim();
    if workspace_dir.is_empty() {
        return;
    }
    let existing_idx = state
        .workspace_overrides
        .iter()
        .position(|entry| workspace_override_matches(&entry.workspace_dir, workspace_dir));
    let workspace_idx = match existing_idx {
        Some(idx) => idx,
        None => {
            state.workspace_overrides.push(WorkspaceSessionState {
                workspace_dir: workspace_dir.to_string(),
                hidden: None,
                pinned: None,
                preferred_targets: None,
            });
            state.workspace_overrides.len() - 1
        }
    };
    {
        let workspace_state = &mut state.workspace_overrides[workspace_idx];
        workspace_state.workspace_dir = workspace_dir.to_string();
        if let Some(hidden) = update.hidden {
            workspace_state.hidden = hidden;
        }
        if let Some(pinned) = update.pinned {
            workspace_state.pinned = pinned;
        }
        if let Some(preferred_targets) = &update.preferred_targets {
            workspace_state.preferred_targets = preferred_targets
                .as_ref()
                .map(|providers| crate::config::normalize_provider_ids(providers.clone()))
                .filter(|providers| !providers.is_empty());
        }
    }
    if workspace_override_is_empty(&state.workspace_overrides[workspace_idx]) {
        state.workspace_overrides.remove(workspace_idx);
    }
}

fn workspace_override_is_empty(workspace_state: &WorkspaceSessionState) -> bool {
    workspace_state.hidden.is_none()
        && workspace_state.pinned.is_none()
        && workspace_state.preferred_targets.is_none()
}

fn parse_string_list(raw: &str, field: &str) -> Result<Vec<String>> {
    serde_json::from_str(raw).with_context(|| format!("Failed to parse {field}"))
}

fn timestamp_from_millis(value: i64) -> Result<chrono::DateTime<Utc>> {
    Utc.timestamp_millis_opt(value)
        .single()
        .with_context(|| format!("Invalid local session state timestamp: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_store() -> (tempfile::TempDir, local_store::LocalSqliteStore) {
        let dir = tempdir().unwrap();
        let store = local_store::LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn sqlite_update_creates_stable_session_and_round_trips_state() {
        let (_dir, mut store) = test_store();
        let update = SessionLocalStateUpdate {
            display_title: Some(Some(" Renamed ".to_string())),
            hidden: Some(true),
            pinned: Some(true),
            notes: Some(Some(" note ".to_string())),
            tags: Some(vec!["a".to_string(), "a".to_string(), " ".to_string()]),
            preferred_targets: Some(vec!["claude".to_string(), "missing".to_string()]),
            compressed_archive_refs: Some(vec!["archive-1".to_string()]),
            workspace_override: None,
        };
        let resolved = update_session_state_with_connection(
            store.connection_mut(),
            "codex",
            "session-1",
            &update,
        )
        .unwrap();
        assert_eq!(resolved.display_title.as_deref(), Some("Renamed"));
        assert!(resolved.hidden);
        assert!(resolved.pinned);
        assert_eq!(resolved.notes.as_deref(), Some("note"));
        assert_eq!(resolved.tags, vec!["a"]);
        assert_eq!(resolved.preferred_targets, vec!["claude"]);
        assert_eq!(resolved.compressed_archive_refs, vec!["archive-1"]);

        let loaded = load_state_store_from_connection(store.connection()).unwrap();
        let loaded = resolve_session_state(&loaded, "codex", "session-1", None);
        assert_eq!(loaded, resolved);
        let session_count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(session_count, 1);
    }

    #[test]
    fn sqlite_workspace_override_updates_and_clears_transactionally() {
        let (_dir, mut store) = test_store();
        update_session_state_with_connection(
            store.connection_mut(),
            "codex",
            "session-1",
            &SessionLocalStateUpdate {
                pinned: Some(true),
                preferred_targets: Some(vec!["claude".to_string()]),
                workspace_override: Some(WorkspaceLocalStateUpdate {
                    workspace_dir: "/tmp/workspace".to_string(),
                    hidden: Some(Some(true)),
                    pinned: Some(Some(false)),
                    preferred_targets: Some(Some(vec!["codex".to_string()])),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let loaded = load_state_store_from_connection(store.connection()).unwrap();
        let resolved = resolve_session_state(&loaded, "codex", "session-1", Some("/tmp/workspace"));
        assert!(resolved.hidden);
        assert!(!resolved.pinned);
        assert_eq!(resolved.preferred_targets, vec!["codex"]);

        update_session_state_with_connection(
            store.connection_mut(),
            "codex",
            "session-1",
            &SessionLocalStateUpdate {
                workspace_override: Some(WorkspaceLocalStateUpdate {
                    workspace_dir: "/tmp/workspace".to_string(),
                    hidden: Some(None),
                    pinned: Some(None),
                    preferred_targets: Some(None),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM workspace_session_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn removing_state_tombstones_managed_session_projection() {
        let (_dir, mut store) = test_store();
        update_session_state_with_connection(
            store.connection_mut(),
            "claude",
            "session-1",
            &SessionLocalStateUpdate {
                hidden: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let canonical = resolve_canonical_session_id(store.connection(), "claude", "session-1")
            .unwrap()
            .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO session_snapshots
                 (session_id, provider_id, updated_at_ms)
                 VALUES (?1, 'claude', 0)",
                [&canonical],
            )
            .unwrap();
        assert_eq!(
            crate::storage::snapshot_store::SnapshotStore::new(store.connection())
                .list_session_snapshots()
                .unwrap()
                .len(),
            1
        );

        remove_session_with_connection(store.connection_mut(), "claude", "session-1").unwrap();

        let sessions: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let local: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM session_local_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        let deleted_at_ms: Option<i64> = store
            .connection()
            .query_row(
                "SELECT deleted_at_ms FROM sessions WHERE id = ?1",
                [&canonical],
                |row| row.get(0),
            )
            .unwrap();
        let visible_sessions: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE deleted_at_ms IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 1);
        assert_eq!(local, 0);
        assert!(deleted_at_ms.is_some());
        assert_eq!(visible_sessions, 0);
        assert!(
            crate::storage::snapshot_store::SnapshotStore::new(store.connection())
                .list_session_snapshots()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            resolve_canonical_session_id(store.connection(), "claude", "session-1")
                .unwrap()
                .as_deref(),
            Some(canonical.as_str())
        );
    }

    #[test]
    fn sqlite_state_does_not_read_legacy_json_files() {
        let (dir, store) = test_store();
        std::fs::write(
            dir.path().join("session_state.json"),
            r#"{
                "version": 2,
                "sessions": {
                    "codex": {
                        "legacy-session": {
                            "locator": {
                                "provider_id": "codex",
                                "session_id": "legacy-session"
                            },
                            "hidden": true,
                            "updated_at": "2026-07-14T00:00:00Z"
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("session_overrides.json"),
            r#"{
                "version": 1,
                "sessions": {
                    "codex": {
                        "legacy-session": {
                            "display_title": "Legacy title",
                            "updated_at": 1783987200000
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = load_state_store_from_connection(store.connection()).unwrap();
        assert!(loaded.sessions.is_empty());
    }

    #[test]
    fn sqlite_state_rejects_invalid_persisted_timestamp() {
        let (_dir, mut store) = test_store();
        update_session_state_with_connection(
            store.connection_mut(),
            "codex",
            "session-1",
            &SessionLocalStateUpdate {
                hidden: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        store
            .connection()
            .execute(
                "UPDATE session_local_state SET updated_at_ms = ?1",
                [i64::MAX],
            )
            .unwrap();

        let error = load_state_store_from_connection(store.connection()).unwrap_err();
        assert!(error
            .to_string()
            .contains("Invalid local session state timestamp"));
    }

    #[test]
    fn in_memory_resolution_keeps_workspace_semantics() {
        let mut store = SessionStateStore::default();
        update_session_state_in_store(
            &mut store,
            "codex",
            "abc",
            &SessionLocalStateUpdate {
                pinned: Some(true),
                preferred_targets: Some(vec!["claude".to_string()]),
                workspace_override: Some(WorkspaceLocalStateUpdate {
                    workspace_dir: "/tmp/workspace".to_string(),
                    hidden: Some(Some(true)),
                    pinned: Some(Some(false)),
                    preferred_targets: Some(Some(vec!["codex".to_string()])),
                }),
                ..Default::default()
            },
        );
        let resolved = resolve_session_state(&store, "codex", "abc", Some("/tmp/workspace"));
        assert!(resolved.hidden);
        assert!(!resolved.pinned);
        assert_eq!(resolved.preferred_targets, vec!["codex"]);
    }
}
