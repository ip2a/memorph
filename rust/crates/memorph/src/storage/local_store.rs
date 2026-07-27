use anyhow::{Context as _, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SCHEMA_VERSION: i64 = 12;

static JOURNAL_MODE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn current_schema_version() -> i64 {
    SCHEMA_VERSION
}

pub struct LocalSqliteStore {
    path: PathBuf,
    conn: Connection,
}

impl LocalSqliteStore {
    pub fn open_default() -> Result<Self> {
        Self::open(database_path()?)
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create memorph data dir: {}", parent.display())
            })?;
        }
        let mut conn = Connection::open(&path)
            .with_context(|| format!("Failed to open memorph DB: {}", path.display()))?;
        configure_connection(&conn).context("Failed to configure memorph DB connection")?;
        apply_schema(&mut conn).context("Failed to initialize memorph DB schema")?;
        Ok(Self { path, conn })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

pub fn database_path() -> Result<PathBuf> {
    Ok(crate::config::memorph_dir()?.join("memorph.db"))
}

pub fn open_database() -> Result<Connection> {
    Ok(LocalSqliteStore::open_default()?.conn)
}

pub(crate) fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)
        .context("Failed to set SQLite busy timeout")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to apply SQLite pragmas")?;
    // SQLite changes journal mode by taking a database-wide lock. Serialize that
    // transition for concurrent stores in this process; the busy timeout covers
    // other processes opening the same database.
    let _journal_mode_guard = JOURNAL_MODE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("SQLite journal mode lock was poisoned"))?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")
        .context("Failed to apply SQLite journal mode")?;
    Ok(())
}

pub(crate) fn apply_schema(conn: &mut Connection) -> Result<()> {
    create_schema_migrations_table(conn)?;
    let applied = applied_migrations(conn)?;
    if applied.contains(&SCHEMA_VERSION) {
        return Ok(());
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start memorph DB schema migration")?;
    let applied = applied_migrations(&tx)?;
    if !applied.contains(&1) {
        tx.execute_batch(V1_SCHEMA)
            .context("Failed to apply memorph DB schema v1")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![1, "local_session_store_v1"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&2) {
        tx.execute_batch(V2_SCHEMA)
            .context("Failed to apply memorph DB schema v2")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![2, "session_activity_query_fields_v2"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&3) {
        tx.execute_batch(V3_SCHEMA)
            .context("Failed to apply memorph DB schema v3")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![3, "hook_event_retention_index_v3"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&4) {
        tx.execute_batch(V4_SCHEMA)
            .context("Failed to apply memorph DB schema v4")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![4, "artifact_manifest_store_v4"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&5) {
        tx.execute_batch(V5_SCHEMA)
            .context("Failed to apply memorph DB schema v5")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![5, "optional_backup_source_path_v5"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&6) {
        tx.execute_batch(V6_SCHEMA)
            .context("Failed to apply memorph DB schema v6")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![6, "backup_restore_attempts_v6"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&7) {
        tx.execute_batch(V7_SCHEMA)
            .context("Failed to apply memorph DB schema v7")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![7, "event_index_turn_links_v7"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    if !applied.contains(&8) {
        tx.execute_batch(V8_SCHEMA)
            .context("Failed to apply memorph DB schema v8")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![8, "source_backed_session_index_v8"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    tx.commit()
        .context("Failed to commit memorph DB schema migration")?;
    apply_bodyless_session_schema(conn)
}

fn apply_bodyless_session_schema(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .context("Failed to disable foreign keys for memorph DB schema v9")?;
    let migration_result = (|| {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("Failed to start memorph DB schema v9 migration")?;
        let applied = applied_migrations(&tx)?;
        if !applied.contains(&9) {
            tx.execute_batch(V9_SCHEMA)
                .context("Failed to apply memorph DB schema v9")?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at_ms)
                 VALUES (?1, ?2, strftime('%s','now') * 1000)",
                params![9, "drop_persisted_session_body_v9"],
            )
            .context("Failed to record memorph DB schema migration")?;
        }
        let foreign_key_violations: i64 = tx
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .context("Failed to validate memorph DB schema v9 foreign keys")?;
        if foreign_key_violations != 0 {
            anyhow::bail!(
                "Memorph DB schema v9 produced {foreign_key_violations} foreign key violations"
            );
        }
        tx.commit()
            .context("Failed to commit memorph DB schema v9 migration")
    })();
    let restore_result = conn
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to restore foreign keys after memorph DB schema v9");
    migration_result?;
    restore_result?;
    apply_skill_schema(conn)
}

fn apply_skill_schema(conn: &mut Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start memorph DB schema v10 migration")?;
    let applied = applied_migrations(&tx)?;
    if !applied.contains(&10) {
        tx.execute_batch(V10_SCHEMA)
            .context("Failed to apply memorph DB schema v10")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![10, "skill_catalog_and_usage_index_v10"],
        )
        .context("Failed to record memorph DB schema migration")?;
    }
    tx.commit()
        .context("Failed to commit memorph DB schema v10 migration")?;
    apply_stats_schema(conn)
}

fn apply_stats_schema(conn: &mut Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("Failed to start memorph DB schema v11 migration")?;
    let applied = applied_migrations(&tx)?;
    if !applied.contains(&11) {
        tx.execute_batch(V11_SCHEMA)
            .context("Failed to apply memorph DB schema v11")?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (?1, ?2, strftime('%s','now') * 1000)",
            params![11, "session_daily_stats_v11"],
        )?;
    }
    tx.commit()
        .context("Failed to commit memorph DB schema v11 migration")?;
    apply_installation_identity_schema(conn)
}

fn apply_installation_identity_schema(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let migration = (|| -> Result<()> {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("Failed to start memorph DB schema v12 migration")?;
        let applied = applied_migrations(&tx)?;
        if !applied.contains(&12) {
            tx.execute_batch(V12_SCHEMA)
                .context("Failed to apply memorph DB schema v12")?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at_ms)
                 VALUES (?1, ?2, strftime('%s','now') * 1000)",
                params![12, "skill_installation_deployment_identity_v12"],
            )?;
        }
        tx.commit()
            .context("Failed to commit memorph DB schema v12 migration")
    })();
    let restore = conn.execute_batch("PRAGMA foreign_keys = ON;");
    migration?;
    restore?;
    let violations: i64 =
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if violations != 0 {
        anyhow::bail!("Memorph DB schema v12 produced {violations} foreign key violations");
    }
    Ok(())
}

fn create_schema_migrations_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL
        );
        ",
    )
    .context("Failed to initialize schema_migrations")
}

fn applied_migrations(conn: &Connection) -> Result<BTreeSet<i64>> {
    let mut stmt = conn
        .prepare("SELECT version FROM schema_migrations")
        .context("Failed to prepare schema migration lookup")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .context("Failed to read schema migrations")?;
    let mut versions = BTreeSet::new();
    for version in rows {
        versions.insert(version.context("Failed to decode schema migration row")?);
    }
    Ok(versions)
}

/// Migration history. V1 creates the original schema including the session
/// body tables (session_turns/session_events/session_event_blocks); V9 drops
/// them. These definitions stay for existing databases that need to replay the
/// full migration chain. New databases apply V1..V8 then V9 in one pass, so the
/// body tables are created and dropped within a single schema initialization.
const V1_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session_index_state (
    provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    file_mtime_ms INTEGER NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    workspace_dir TEXT,
    created_at_ms INTEGER,
    last_active_at_ms INTEGER,
    source_title TEXT,
    event_count INTEGER NOT NULL,
    message_count INTEGER NOT NULL,
    indexed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (provider_id, source_path)
);

CREATE TABLE IF NOT EXISTS session_event_index (
    provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    file_mtime_ms INTEGER NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    event_index INTEGER NOT NULL,
    byte_offset INTEGER NOT NULL,
    byte_length INTEGER NOT NULL,
    line_no INTEGER NOT NULL,
    PRIMARY KEY (provider_id, source_path, file_mtime_ms, file_size_bytes, event_index)
);

CREATE INDEX IF NOT EXISTS idx_session_event_index_page
    ON session_event_index(provider_id, source_path, file_mtime_ms, file_size_bytes, event_index);

CREATE TABLE IF NOT EXISTS session_sources (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_session_id TEXT,
    source_path TEXT NOT NULL,
    workspace_dir TEXT,
    storage_shape TEXT,
    file_mtime_ms INTEGER,
    file_size_bytes INTEGER,
    content_hash TEXT,
    source_cursor TEXT,
    scan_generation INTEGER NOT NULL DEFAULT 0,
    provider_schema_version TEXT,
    first_seen_at_ms INTEGER NOT NULL,
    last_seen_at_ms INTEGER NOT NULL,
    UNIQUE(provider_id, source_path)
);

CREATE INDEX IF NOT EXISTS idx_session_sources_provider_workspace
    ON session_sources(provider_id, workspace_dir, last_seen_at_ms);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_session_id TEXT,
    primary_source_id TEXT,
    workspace_dir TEXT,
    title TEXT,
    status TEXT NOT NULL DEFAULT 'unknown',
    created_at_ms INTEGER,
    updated_at_ms INTEGER,
    last_active_at_ms INTEGER,
    event_count INTEGER NOT NULL DEFAULT 0,
    turn_count INTEGER NOT NULL DEFAULT 0,
    projection_version INTEGER NOT NULL DEFAULT 0,
    deleted_at_ms INTEGER,
    FOREIGN KEY(primary_source_id) REFERENCES session_sources(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_provider_last_active
    ON sessions(provider_id, last_active_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_sessions_workspace_last_active
    ON sessions(workspace_dir, last_active_at_ms DESC);

CREATE TABLE IF NOT EXISTS session_aliases (
    alias_kind TEXT NOT NULL,
    alias_value TEXT NOT NULL,
    session_id TEXT NOT NULL,
    provider_id TEXT,
    source_id TEXT,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (alias_kind, alias_value),
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(source_id) REFERENCES session_sources(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_session_aliases_session
    ON session_aliases(session_id);

CREATE TABLE IF NOT EXISTS session_snapshots (
    session_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    title TEXT,
    display_title TEXT,
    workspace_dir TEXT,
    status TEXT NOT NULL DEFAULT 'unknown',
    last_active_at_ms INTEGER,
    event_count INTEGER NOT NULL DEFAULT 0,
    turn_count INTEGER NOT NULL DEFAULT 0,
    flags_json TEXT NOT NULL DEFAULT '{}',
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    projection_version INTEGER NOT NULL DEFAULT 0,
    source_fingerprint TEXT,
    stale INTEGER NOT NULL DEFAULT 0,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_session_snapshots_provider_last_active
    ON session_snapshots(provider_id, last_active_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_snapshots_workspace_last_active
    ON session_snapshots(workspace_dir, last_active_at_ms DESC);

CREATE TABLE IF NOT EXISTS session_turns (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    provider_turn_id TEXT,
    status TEXT NOT NULL DEFAULT 'unknown',
    confidence TEXT NOT NULL DEFAULT 'unknown',
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    source_start_cursor TEXT,
    source_end_cursor TEXT,
    source_range_json TEXT NOT NULL DEFAULT '{}',
    turn_order INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_turns_order
    ON session_turns(session_id, turn_order);

CREATE TABLE IF NOT EXISTS session_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    turn_id TEXT,
    provider_event_id TEXT,
    role TEXT,
    kind TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'visible',
    timestamp_ms INTEGER,
    source_order INTEGER NOT NULL,
    stable_cursor TEXT NOT NULL,
    source_id TEXT,
    source_cursor TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(turn_id) REFERENCES session_turns(id) ON DELETE SET NULL,
    FOREIGN KEY(source_id) REFERENCES session_sources(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_events_order
    ON session_events(session_id, timestamp_ms, source_order, stable_cursor);
CREATE INDEX IF NOT EXISTS idx_session_events_turn
    ON session_events(turn_id, source_order);

CREATE TABLE IF NOT EXISTS session_event_blocks (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    block_order INTEGER NOT NULL,
    block_kind TEXT NOT NULL,
    fidelity TEXT NOT NULL DEFAULT 'preserved',
    content_text TEXT,
    content_json TEXT,
    artifact_id TEXT,
    preview TEXT,
    byte_size INTEGER,
    content_hash TEXT,
    provider_path TEXT,
    FOREIGN KEY(event_id) REFERENCES session_events(id) ON DELETE CASCADE,
    FOREIGN KEY(artifact_id) REFERENCES artifact_manifests(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_event_blocks_order
    ON session_event_blocks(event_id, block_order);

CREATE TABLE IF NOT EXISTS projection_reports (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    provider_id TEXT NOT NULL,
    source_id TEXT,
    operation_kind TEXT NOT NULL,
    projection_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY(source_id) REFERENCES session_sources(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_projection_reports_session
    ON projection_reports(session_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS projection_report_items (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL,
    item_order INTEGER NOT NULL,
    disposition TEXT NOT NULL,
    scope TEXT NOT NULL,
    field_path TEXT,
    reason TEXT,
    details_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(report_id) REFERENCES projection_reports(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_projection_report_items_report
    ON projection_report_items(report_id, item_order);

CREATE TABLE IF NOT EXISTS session_local_state (
    session_id TEXT PRIMARY KEY,
    display_title TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    hidden INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0,
    notes TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    preferred_targets_json TEXT NOT NULL DEFAULT '[]',
    compressed_archive_refs_json TEXT NOT NULL DEFAULT '[]',
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workspace_session_state (
    session_id TEXT NOT NULL,
    workspace_dir TEXT NOT NULL,
    hidden INTEGER,
    pinned INTEGER,
    preferred_targets_json TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(session_id, workspace_dir),
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sync_groups (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    source_provider TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
);

CREATE TABLE IF NOT EXISTS sync_holdings (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    session_id TEXT,
    provider_session_id TEXT NOT NULL,
    target_dir TEXT,
    created_at_ms INTEGER NOT NULL,
    last_active_at_ms INTEGER,
    last_sync_at_ms INTEGER,
    last_sync_from TEXT,
    last_error TEXT,
    FOREIGN KEY(group_id) REFERENCES sync_groups(id) ON DELETE CASCADE,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_holdings_group
    ON sync_holdings(group_id);

CREATE TABLE IF NOT EXISTS sync_runs (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    source_holding_id TEXT,
    status TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    result_json TEXT NOT NULL DEFAULT '{}',
    error TEXT,
    FOREIGN KEY(group_id) REFERENCES sync_groups(id) ON DELETE CASCADE,
    FOREIGN KEY(source_holding_id) REFERENCES sync_holdings(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_runs_group_started
    ON sync_runs(group_id, started_at_ms DESC);

CREATE TABLE IF NOT EXISTS session_activity (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    provider_id TEXT,
    workspace_dir TEXT,
    operation_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    actor TEXT,
    summary TEXT,
    details_json TEXT NOT NULL DEFAULT '{}',
    error TEXT,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_session_activity_session_started
    ON session_activity(session_id, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_provider_started
    ON session_activity(provider_id, started_at_ms DESC);

CREATE TABLE IF NOT EXISTS runtime_endpoints (
    id TEXT PRIMARY KEY,
    runtime_kind TEXT NOT NULL,
    pid INTEGER,
    host TEXT,
    port INTEGER,
    base_url TEXT,
    token_hash TEXT,
    published_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    last_seen_at_ms INTEGER NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_runtime_endpoints_kind_seen
    ON runtime_endpoints(runtime_kind, last_seen_at_ms DESC);

CREATE TABLE IF NOT EXISTS runtime_session_observations (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_session_id TEXT,
    session_id TEXT,
    workspace_dir TEXT,
    status TEXT NOT NULL,
    correlation_id TEXT,
    observed_at_ms INTEGER NOT NULL,
    recent_activity_at_ms INTEGER,
    details_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_runtime_session_observations_session
    ON runtime_session_observations(session_id, observed_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_runtime_session_observations_provider
    ON runtime_session_observations(provider_id, provider_session_id, observed_at_ms DESC);

CREATE TABLE IF NOT EXISTS hook_events (
    id TEXT PRIMARY KEY,
    provider_id TEXT,
    provider_session_id TEXT,
    session_id TEXT,
    event_name TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL,
    correlation_id TEXT,
    payload_json TEXT,
    -- Schema debt: payload_artifact_id has no production write/read path. V9
    -- nulls existing references. Kept to avoid a destructive ALTER on existing
    -- databases; cleanup deferred until hook payload projection is implemented.
    payload_artifact_id TEXT,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY(payload_artifact_id) REFERENCES artifact_manifests(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_hook_events_provider_seen
    ON hook_events(provider_id, observed_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_hook_events_session_seen
    ON hook_events(session_id, observed_at_ms DESC);

CREATE TABLE IF NOT EXISTS hook_errors (
    id TEXT PRIMARY KEY,
    provider_id TEXT,
    session_id TEXT,
    scope TEXT NOT NULL,
    message TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_hook_errors_seen
    ON hook_errors(observed_at_ms DESC);

CREATE TABLE IF NOT EXISTS artifact_manifests (
    id TEXT PRIMARY KEY,
    artifact_kind TEXT NOT NULL,
    session_id TEXT,
    event_id TEXT,
    block_id TEXT,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    mime_type TEXT,
    format TEXT,
    created_at_ms INTEGER NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY(event_id) REFERENCES session_events(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_artifact_manifests_session
    ON artifact_manifests(session_id, created_at_ms DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_artifact_manifests_hash_kind
    ON artifact_manifests(artifact_kind, content_hash, path);

CREATE TABLE IF NOT EXISTS backups (
    id TEXT PRIMARY KEY,
    operation_id TEXT,
    provider_id TEXT,
    session_id TEXT,
    source_path TEXT NOT NULL,
    backup_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    restore_hint TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_backups_session_created
    ON backups(session_id, created_at_ms DESC);
"#;

const V2_SCHEMA: &str = r#"
ALTER TABLE session_activity ADD COLUMN provider_session_id TEXT;

CREATE INDEX IF NOT EXISTS idx_session_activity_provider_session_started
    ON session_activity(provider_id, provider_session_id, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_workspace_started
    ON session_activity(workspace_dir, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_operation_started
    ON session_activity(operation_kind, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_status_started
    ON session_activity(status, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_actor_started
    ON session_activity(actor, started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_session_activity_started
    ON session_activity(started_at_ms DESC);
"#;

const V3_SCHEMA: &str = r#"
CREATE INDEX IF NOT EXISTS idx_hook_events_seen
    ON hook_events(observed_at_ms DESC);
"#;

const V4_SCHEMA: &str = r#"
ALTER TABLE artifact_manifests ADD COLUMN operation_id TEXT;
ALTER TABLE artifact_manifests ADD COLUMN provider_id TEXT;
ALTER TABLE artifact_manifests ADD COLUMN provider_session_id TEXT;
ALTER TABLE artifact_manifests ADD COLUMN projection_report_id TEXT
    REFERENCES projection_reports(id) ON DELETE SET NULL;
ALTER TABLE artifact_manifests ADD COLUMN storage_kind TEXT NOT NULL DEFAULT 'unknown';

INSERT OR IGNORE INTO artifact_manifests
    (id, artifact_kind, session_id, path, content_hash, byte_size, created_at_ms,
     metadata_json, operation_id, provider_id, storage_kind)
SELECT
    'legacy-backup-artifact:' || id,
    'session_backup',
    session_id,
    backup_path,
    content_hash,
    byte_size,
    created_at_ms,
    metadata_json,
    operation_id,
    provider_id,
    'unknown'
FROM backups;

ALTER TABLE backups RENAME TO backups_v3;
DROP INDEX IF EXISTS idx_backups_session_created;

CREATE TABLE backups (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL UNIQUE,
    operation_id TEXT,
    provider_id TEXT,
    provider_session_id TEXT,
    session_id TEXT,
    source_path TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    restore_hint TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(artifact_id) REFERENCES artifact_manifests(id) ON DELETE RESTRICT,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

INSERT INTO backups
    (id, artifact_id, operation_id, provider_id, provider_session_id, session_id,
     source_path, created_at_ms, restore_hint, metadata_json)
SELECT
    legacy.id,
    (
        SELECT artifact.id
        FROM artifact_manifests artifact
        WHERE artifact.artifact_kind = 'session_backup'
          AND artifact.path = legacy.backup_path
          AND artifact.content_hash = legacy.content_hash
        ORDER BY artifact.created_at_ms, artifact.id
        LIMIT 1
    ),
    legacy.operation_id,
    legacy.provider_id,
    NULL,
    legacy.session_id,
    legacy.source_path,
    legacy.created_at_ms,
    legacy.restore_hint,
    legacy.metadata_json
FROM backups_v3 legacy;

DROP TABLE backups_v3;

CREATE INDEX idx_backups_session_created
    ON backups(session_id, created_at_ms DESC);
CREATE INDEX idx_backups_provider_session_created
    ON backups(provider_id, provider_session_id, created_at_ms DESC);
CREATE INDEX idx_backups_operation
    ON backups(operation_id);

DROP INDEX IF EXISTS idx_artifact_manifests_hash_kind;
CREATE UNIQUE INDEX idx_artifact_manifests_registration
    ON artifact_manifests(
        artifact_kind,
        path,
        content_hash,
        COALESCE(operation_id, '')
    );
CREATE INDEX idx_artifact_manifests_operation
    ON artifact_manifests(operation_id);
CREATE INDEX idx_artifact_manifests_provider_session
    ON artifact_manifests(provider_id, provider_session_id, created_at_ms DESC);
CREATE INDEX idx_artifact_manifests_projection_report
    ON artifact_manifests(projection_report_id);
"#;

const V5_SCHEMA: &str = r#"
ALTER TABLE backups RENAME TO backups_v4;
DROP INDEX IF EXISTS idx_backups_session_created;
DROP INDEX IF EXISTS idx_backups_provider_session_created;
DROP INDEX IF EXISTS idx_backups_operation;

CREATE TABLE backups (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL UNIQUE,
    operation_id TEXT,
    provider_id TEXT,
    provider_session_id TEXT,
    session_id TEXT,
    source_path TEXT,
    created_at_ms INTEGER NOT NULL,
    restore_hint TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(artifact_id) REFERENCES artifact_manifests(id) ON DELETE RESTRICT,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

INSERT INTO backups
    (id, artifact_id, operation_id, provider_id, provider_session_id, session_id,
     source_path, created_at_ms, restore_hint, metadata_json)
SELECT
    id,
    artifact_id,
    operation_id,
    provider_id,
    provider_session_id,
    session_id,
    source_path,
    created_at_ms,
    restore_hint,
    metadata_json
FROM backups_v4;

DROP TABLE backups_v4;

CREATE INDEX idx_backups_session_created
    ON backups(session_id, created_at_ms DESC);
CREATE INDEX idx_backups_provider_session_created
    ON backups(provider_id, provider_session_id, created_at_ms DESC);
CREATE INDEX idx_backups_operation
    ON backups(operation_id);
"#;

const V6_SCHEMA: &str = r#"
CREATE TABLE backup_restores (
    id TEXT PRIMARY KEY,
    backup_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('running', 'success', 'failed')),
    actor TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    error TEXT,
    FOREIGN KEY(backup_id) REFERENCES backups(id) ON DELETE RESTRICT
);

CREATE INDEX idx_backup_restores_backup_started
    ON backup_restores(backup_id, started_at_ms DESC, id DESC);
CREATE INDEX idx_backup_restores_status_started
    ON backup_restores(status, started_at_ms DESC);
"#;

const V7_SCHEMA: &str = r#"
ALTER TABLE session_event_index ADD COLUMN provider_turn_id TEXT;
ALTER TABLE session_event_index ADD COLUMN turn_index INTEGER;
ALTER TABLE session_event_index ADD COLUMN turn_boundary TEXT
    CHECK(turn_boundary IS NULL OR turn_boundary IN ('started', 'completed', 'failed', 'interrupted'));

CREATE INDEX idx_session_event_index_turn
    ON session_event_index(provider_id, source_path, provider_turn_id, event_index);
"#;

const V8_SCHEMA: &str = r#"
ALTER TABLE session_snapshots ADD COLUMN message_count INTEGER;
ALTER TABLE session_snapshots ADD COLUMN counts_complete INTEGER NOT NULL DEFAULT 0
    CHECK(counts_complete IN (0, 1));

CREATE INDEX idx_session_snapshots_provider_workspace_recent
    ON session_snapshots(provider_id, workspace_dir, last_active_at_ms DESC);
"#;

/// Drop persisted session body. ORDER is dependency-safe:
/// session_event_blocks -> session_events -> session_turns. FK is OFF for the
/// whole block, but the order is correct even if FK were ON. Also rebuilds
/// artifact_manifests without event_id/block_id and purges event_payload rows.
const V9_SCHEMA: &str = r#"
UPDATE hook_events
SET payload_artifact_id = NULL
WHERE payload_artifact_id IN (
    SELECT id
    FROM artifact_manifests
    WHERE artifact_kind = 'event_payload'
);

DROP TABLE session_event_blocks;
DROP TABLE session_events;
DROP TABLE session_turns;

CREATE TABLE artifact_manifests_v9 (
    id TEXT PRIMARY KEY,
    artifact_kind TEXT NOT NULL,
    session_id TEXT,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    mime_type TEXT,
    format TEXT,
    created_at_ms INTEGER NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    operation_id TEXT,
    provider_id TEXT,
    provider_session_id TEXT,
    projection_report_id TEXT,
    storage_kind TEXT NOT NULL DEFAULT 'unknown',
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY(projection_report_id) REFERENCES projection_reports(id) ON DELETE SET NULL
);

INSERT INTO artifact_manifests_v9
    (id, artifact_kind, session_id, path, content_hash, byte_size, mime_type, format,
     created_at_ms, metadata_json, operation_id, provider_id, provider_session_id,
     projection_report_id, storage_kind)
SELECT
    id, artifact_kind, session_id, path, content_hash, byte_size, mime_type, format,
    created_at_ms, metadata_json, operation_id, provider_id, provider_session_id,
    projection_report_id, storage_kind
FROM artifact_manifests
WHERE artifact_kind <> 'event_payload';

DROP TABLE artifact_manifests;
ALTER TABLE artifact_manifests_v9 RENAME TO artifact_manifests;

CREATE UNIQUE INDEX idx_artifact_manifests_registration
    ON artifact_manifests(
        artifact_kind,
        path,
        content_hash,
        COALESCE(operation_id, '')
    );
CREATE INDEX idx_artifact_manifests_session
    ON artifact_manifests(session_id, created_at_ms DESC);
CREATE INDEX idx_artifact_manifests_operation
    ON artifact_manifests(operation_id);
CREATE INDEX idx_artifact_manifests_provider_session
    ON artifact_manifests(provider_id, provider_session_id, created_at_ms DESC);
CREATE INDEX idx_artifact_manifests_projection_report
    ON artifact_manifests(projection_report_id);
"#;

const V10_SCHEMA: &str = r#"
CREATE TABLE skill_catalog (
    id TEXT PRIMARY KEY,
    canonical_name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    description TEXT,
    version TEXT,
    author TEXT,
    entry_file_name TEXT NOT NULL DEFAULT 'SKILL.md',
    entry_content_hash TEXT NOT NULL,
    bundle_content_hash TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    trigger_terms_json TEXT NOT NULL DEFAULT '[]',
    section_index_json TEXT NOT NULL DEFAULT '[]',
    file_manifest_json TEXT NOT NULL DEFAULT '[]',
    file_count INTEGER NOT NULL DEFAULT 0 CHECK(file_count >= 0),
    total_bytes INTEGER NOT NULL DEFAULT 0 CHECK(total_bytes >= 0),
    metadata_bytes INTEGER NOT NULL DEFAULT 0 CHECK(metadata_bytes >= 0),
    body_bytes INTEGER NOT NULL DEFAULT 0 CHECK(body_bytes >= 0),
    auxiliary_bytes INTEGER NOT NULL DEFAULT 0 CHECK(auxiliary_bytes >= 0),
    first_seen_at_ms INTEGER NOT NULL,
    last_scanned_at_ms INTEGER NOT NULL,
    missing_since_ms INTEGER,
    scan_error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_skill_catalog_normalized_name ON skill_catalog(normalized_name);
CREATE INDEX idx_skill_catalog_last_scanned ON skill_catalog(last_scanned_at_ms DESC);
CREATE INDEX idx_skill_catalog_missing ON skill_catalog(missing_since_ms);

CREATE TABLE skill_installations (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('global', 'project')),
    workspace_dir TEXT,
    install_path TEXT NOT NULL,
    canonical_install_path TEXT NOT NULL,
    install_kind TEXT NOT NULL CHECK(install_kind IN ('directory', 'symlink', 'managed-copy')),
    symlink_target TEXT,
    source_path TEXT,
    managed_marker_present INTEGER NOT NULL DEFAULT 0 CHECK(managed_marker_present IN (0, 1)),
    link_status TEXT NOT NULL DEFAULT 'not-applicable'
        CHECK(link_status IN ('not-applicable', 'valid', 'broken', 'outside-allowed-root', 'loop')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK(status IN ('active', 'missing', 'removed', 'error')),
    bundle_content_hash TEXT NOT NULL,
    discovered_at_ms INTEGER NOT NULL,
    last_verified_at_ms INTEGER NOT NULL,
    removed_at_ms INTEGER,
    error_text TEXT,
    FOREIGN KEY(skill_id) REFERENCES skill_catalog(id) ON DELETE RESTRICT,
    UNIQUE(provider_id, canonical_install_path)
);
CREATE INDEX idx_skill_installations_skill ON skill_installations(skill_id, status);
CREATE INDEX idx_skill_installations_provider_scope
    ON skill_installations(provider_id, scope_kind, workspace_dir, status);
CREATE INDEX idx_skill_installations_link_status ON skill_installations(link_status, status);

CREATE TABLE skill_scan_state (
    state_key TEXT PRIMARY KEY,
    state_kind TEXT NOT NULL CHECK(state_kind IN ('skill-root', 'session-source', 'aggregate')),
    provider_id TEXT,
    source_path TEXT,
    source_fingerprint TEXT,
    source_cursor TEXT,
    last_started_at_ms INTEGER,
    last_completed_at_ms INTEGER,
    last_full_scan_at_ms INTEGER,
    scan_generation INTEGER NOT NULL DEFAULT 0,
    completeness_status TEXT NOT NULL DEFAULT 'unknown'
        CHECK(completeness_status IN ('unknown', 'partial', 'complete', 'error')),
    earliest_indexed_at_ms INTEGER,
    latest_indexed_at_ms INTEGER,
    items_seen INTEGER NOT NULL DEFAULT 0,
    error_text TEXT,
    details_json TEXT NOT NULL DEFAULT '{}',
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_skill_scan_state_kind_provider ON skill_scan_state(state_kind, provider_id);
CREATE INDEX idx_skill_scan_state_completeness
    ON skill_scan_state(completeness_status, updated_at_ms DESC);

CREATE TABLE skill_invocations (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    installation_id TEXT,
    session_id TEXT NOT NULL,
    source_id TEXT,
    event_id TEXT,
    provider_id TEXT NOT NULL,
    workspace_dir TEXT,
    invoked_at_ms INTEGER NOT NULL,
    detection_kind TEXT NOT NULL
        CHECK(detection_kind IN ('explicit-tool', 'explicit-name', 'entry-path', 'bundle-path', 'content-evidence')),
    confidence TEXT NOT NULL CHECK(confidence IN ('high', 'medium', 'low')),
    evidence_text TEXT,
    evidence_path TEXT,
    token_count INTEGER,
    source_fingerprint TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(skill_id) REFERENCES skill_catalog(id) ON DELETE RESTRICT,
    FOREIGN KEY(installation_id) REFERENCES skill_installations(id) ON DELETE SET NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(source_id) REFERENCES session_sources(id) ON DELETE CASCADE,
    UNIQUE(session_id, event_id, skill_id, detection_kind, source_fingerprint)
);
CREATE INDEX idx_skill_invocations_skill_time ON skill_invocations(skill_id, invoked_at_ms DESC);
CREATE INDEX idx_skill_invocations_session ON skill_invocations(session_id, invoked_at_ms);
CREATE INDEX idx_skill_invocations_provider_time
    ON skill_invocations(provider_id, invoked_at_ms DESC);
CREATE INDEX idx_skill_invocations_workspace_time
    ON skill_invocations(workspace_dir, invoked_at_ms DESC);
CREATE INDEX idx_skill_invocations_source ON skill_invocations(source_id);

CREATE TABLE skill_usage_daily (
    usage_date TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    workspace_key TEXT NOT NULL DEFAULT '',
    invocation_count INTEGER NOT NULL DEFAULT 0 CHECK(invocation_count >= 0),
    session_count INTEGER NOT NULL DEFAULT 0 CHECK(session_count >= 0),
    token_count INTEGER,
    high_confidence_count INTEGER NOT NULL DEFAULT 0,
    medium_confidence_count INTEGER NOT NULL DEFAULT 0,
    low_confidence_count INTEGER NOT NULL DEFAULT 0,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(usage_date, skill_id, provider_id, workspace_key),
    FOREIGN KEY(skill_id) REFERENCES skill_catalog(id) ON DELETE RESTRICT
);
CREATE INDEX idx_skill_usage_daily_date ON skill_usage_daily(usage_date DESC);
CREATE INDEX idx_skill_usage_daily_skill_date ON skill_usage_daily(skill_id, usage_date DESC);
CREATE INDEX idx_skill_usage_daily_provider_date
    ON skill_usage_daily(provider_id, usage_date DESC);

CREATE TABLE skill_coverage_observations (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    target_kind TEXT NOT NULL
        CHECK(target_kind IN ('section', 'script', 'reference', 'asset', 'other-file')),
    target_key TEXT NOT NULL,
    target_path TEXT,
    section_title TEXT,
    match_kind TEXT NOT NULL
        CHECK(match_kind IN ('exact-path', 'normalized-path', 'section-anchor', 'unique-basename', 'content-reference')),
    confidence TEXT NOT NULL CHECK(confidence IN ('high', 'medium', 'low')),
    observed_at_ms INTEGER NOT NULL,
    evidence_text TEXT,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(skill_id) REFERENCES skill_catalog(id) ON DELETE RESTRICT,
    FOREIGN KEY(invocation_id) REFERENCES skill_invocations(id) ON DELETE CASCADE,
    UNIQUE(invocation_id, target_kind, target_key, match_kind)
);
CREATE INDEX idx_skill_coverage_skill_target
    ON skill_coverage_observations(skill_id, target_kind, target_key, observed_at_ms DESC);
CREATE INDEX idx_skill_coverage_invocation ON skill_coverage_observations(invocation_id);
"#;

const V11_SCHEMA: &str = r#"
CREATE TABLE session_daily_stats (
    session_id TEXT NOT NULL,
    day_start_ms INTEGER NOT NULL,
    event_count INTEGER NOT NULL,
    message_count INTEGER NOT NULL,
    PRIMARY KEY (session_id, day_start_ms),
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
CREATE INDEX idx_session_daily_stats_day ON session_daily_stats(day_start_ms);
UPDATE session_snapshots SET counts_complete = 0;
"#;

const V12_SCHEMA: &str = r#"
CREATE TABLE skill_installations_v12 (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('global', 'project')),
    workspace_dir TEXT,
    install_path TEXT NOT NULL,
    canonical_install_path TEXT NOT NULL,
    install_kind TEXT NOT NULL CHECK(install_kind IN ('directory', 'symlink', 'managed-copy')),
    symlink_target TEXT,
    source_path TEXT,
    managed_marker_present INTEGER NOT NULL DEFAULT 0 CHECK(managed_marker_present IN (0, 1)),
    link_status TEXT NOT NULL DEFAULT 'not-applicable'
        CHECK(link_status IN ('not-applicable', 'valid', 'broken', 'outside-allowed-root', 'loop')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK(status IN ('active', 'missing', 'removed', 'error')),
    bundle_content_hash TEXT NOT NULL,
    discovered_at_ms INTEGER NOT NULL,
    last_verified_at_ms INTEGER NOT NULL,
    removed_at_ms INTEGER,
    error_text TEXT,
    FOREIGN KEY(skill_id) REFERENCES skill_catalog(id) ON DELETE RESTRICT,
    UNIQUE(provider_id, install_path)
);
INSERT INTO skill_installations_v12 SELECT * FROM skill_installations;
DROP TABLE skill_installations;
ALTER TABLE skill_installations_v12 RENAME TO skill_installations;
CREATE INDEX idx_skill_installations_skill ON skill_installations(skill_id, status);
CREATE INDEX idx_skill_installations_provider_scope
    ON skill_installations(provider_id, scope_kind, workspace_dir, status);
CREATE INDEX idx_skill_installations_link_status ON skill_installations(link_status, status);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(()),
        )
        .is_ok()
    }

    #[test]
    fn initializes_schema_and_connection_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memorph.db");
        let store = LocalSqliteStore::open(&path).unwrap();
        let conn = store.connection();

        assert_eq!(store.path(), path.as_path());
        assert!(table_exists(conn, "schema_migrations"));
        assert!(table_exists(conn, "session_sources"));
        assert!(table_exists(conn, "session_snapshots"));
        assert!(!table_exists(conn, "session_turns"));
        assert!(!table_exists(conn, "session_events"));
        assert!(!table_exists(conn, "session_event_blocks"));
        assert!(table_exists(conn, "session_activity"));
        assert!(table_exists(conn, "artifact_manifests"));
        assert!(table_exists(conn, "backups"));
        assert!(table_exists(conn, "backup_restores"));
        assert!(table_exists(conn, "skill_catalog"));
        assert!(table_exists(conn, "skill_installations"));
        assert!(table_exists(conn, "skill_scan_state"));
        assert!(table_exists(conn, "skill_invocations"));
        assert!(table_exists(conn, "skill_usage_daily"));
        assert!(table_exists(conn, "skill_coverage_observations"));

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        let busy_timeout_ms: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();

        assert_eq!(foreign_keys, 1);
        assert!(busy_timeout_ms >= 5000);
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    #[test]
    fn skill_schema_enforces_installation_and_usage_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        let conn = store.connection();
        conn.execute(
            "INSERT INTO skill_catalog (
                id, canonical_name, normalized_name, entry_content_hash,
                bundle_content_hash, first_seen_at_ms, last_scanned_at_ms,
                created_at_ms, updated_at_ms
             ) VALUES ('skill:test', 'Test', 'test', 'entry', 'bundle', 1, 1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO skill_installations (
                id, skill_id, provider_id, scope_kind, install_path,
                canonical_install_path, install_kind, bundle_content_hash,
                discovered_at_ms, last_verified_at_ms
             ) VALUES ('install:test', 'skill:test', 'codex', 'global', '/tmp/test',
                       '/tmp/test', 'directory', 'bundle', 1, 1)",
            [],
        )
        .unwrap();

        let duplicate = conn.execute(
            "INSERT INTO skill_installations (
                id, skill_id, provider_id, scope_kind, install_path,
                canonical_install_path, install_kind, bundle_content_hash,
                discovered_at_ms, last_verified_at_ms
             ) VALUES ('install:duplicate', 'skill:test', 'codex', 'global', '/tmp/test',
                       '/tmp/test', 'directory', 'bundle', 1, 1)",
            [],
        );
        assert!(duplicate.is_err());

        conn.execute(
            "INSERT INTO skill_installations (
                id, skill_id, provider_id, scope_kind, install_path,
                canonical_install_path, install_kind, bundle_content_hash,
                discovered_at_ms, last_verified_at_ms
             ) VALUES ('install:project', 'skill:test', 'codex', 'project', '/project/.codex/skills/test',
                       '/tmp/test', 'symlink', 'bundle', 1, 1)",
            [],
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill_installations WHERE canonical_install_path = '/tmp/test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn schema_migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memorph.db");

        drop(LocalSqliteStore::open(&path).unwrap());
        let store = LocalSqliteStore::open(&path).unwrap();
        let migration_count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let max_version: i64 = store
            .connection()
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(migration_count, SCHEMA_VERSION);
        assert_eq!(max_version, SCHEMA_VERSION);
    }

    #[test]
    fn concurrent_initialization_applies_schema_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memorph.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    LocalSqliteStore::open(path)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap().unwrap();
        }

        let store = LocalSqliteStore::open(path).unwrap();
        let migration_count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v1_schema_to_latest() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (1, 'local_session_store_v1', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        let provider_session_id_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM pragma_table_info('session_activity')
                    WHERE name = 'provider_session_id'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(provider_session_id_exists);
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_body_tables_to_bodyless_v9() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES (1, 'local_session_store_v1', 0)",
            [],
        )
        .unwrap();

        // Seed a full body chain plus both an event_payload manifest and a
        // non-event_payload manifest, and hook events referencing each. This
        // exercises the v9 DROP of the body tables, the event_payload manifest
        // purge, the hook reference nulling, and survival of other manifests.
        conn.execute_batch(
            r#"
            INSERT INTO session_sources (id, provider_id, provider_session_id, source_path, first_seen_at_ms, last_seen_at_ms)
            VALUES ('src_1', 'claude', 'p_sess_1', '/tmp/session.jsonl', 1, 1);

            INSERT INTO sessions (id, provider_id, provider_session_id, primary_source_id)
            VALUES ('sess_1', 'claude', 'p_sess_1', 'src_1');

            INSERT INTO session_turns (id, session_id, turn_order)
            VALUES ('turn_1', 'sess_1', 0);

            INSERT INTO session_events (id, session_id, turn_id, kind, source_order, stable_cursor)
            VALUES ('event_1', 'sess_1', 'turn_1', 'message', 0, 'cursor_0');

            INSERT INTO artifact_manifests (id, artifact_kind, session_id, event_id, path, content_hash, byte_size, created_at_ms)
            VALUES ('art_payload', 'event_payload', 'sess_1', 'event_1', '/tmp/blob_payload.bin', 'hash_payload', 11, 1);

            INSERT INTO artifact_manifests (id, artifact_kind, session_id, path, content_hash, byte_size, created_at_ms)
            VALUES ('art_backup', 'session_backup', 'sess_1', '/tmp/blob_backup.bin', 'hash_backup', 22, 1);

            INSERT INTO session_event_blocks (id, event_id, block_order, block_kind, artifact_id)
            VALUES ('block_1', 'event_1', 0, 'content', 'art_payload');

            INSERT INTO hook_events (id, provider_id, provider_session_id, session_id, event_name, observed_at_ms, payload_artifact_id)
            VALUES ('hook_payload', 'claude', 'p_sess_1', 'sess_1', 'session.start', 1, 'art_payload');

            INSERT INTO hook_events (id, provider_id, provider_session_id, session_id, event_name, observed_at_ms, payload_artifact_id)
            VALUES ('hook_backup', 'claude', 'p_sess_1', 'sess_1', 'session.stop', 2, 'art_backup');
            "#,
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        assert!(!table_exists(&conn, "session_turns"));
        assert!(!table_exists(&conn, "session_events"));
        assert!(!table_exists(&conn, "session_event_blocks"));

        let payload_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_manifests WHERE id = 'art_payload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(payload_remaining, 0);

        let backup_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_manifests WHERE id = 'art_backup'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backup_remaining, 1);

        let nulled_hook: Option<String> = conn
            .query_row(
                "SELECT payload_artifact_id FROM hook_events WHERE id = 'hook_payload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(nulled_hook.is_none());

        let preserved_hook: Option<String> = conn
            .query_row(
                "SELECT payload_artifact_id FROM hook_events WHERE id = 'hook_backup'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved_hook.as_deref(), Some("art_backup"));

        // The rebuilt manifest table no longer carries the body-only columns.
        let event_id_col_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('artifact_manifests')
                    WHERE name = 'event_id'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!event_id_col_exists);

        let foreign_key_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_violations, 0);

        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v2_schema_to_v3() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES
             (1, 'local_session_store_v1', 0),
             (2, 'session_activity_query_fields_v2', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        let retention_index_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM sqlite_master
                    WHERE type = 'index' AND name = 'idx_hook_events_seen'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(retention_index_exists);
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v3_artifacts_and_backups_to_v4() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        conn.execute_batch(V3_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO backups
             (id, operation_id, provider_id, session_id, source_path, backup_path,
              content_hash, byte_size, created_at_ms, restore_hint, metadata_json)
             VALUES
             ('backup-1', 'operation-1', 'codex', NULL, '/source/session.jsonl',
              '/backup/session.jsonl', 'legacy-hash', 42, 1000, 'copy back', '{\"v\":1}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES
             (1, 'local_session_store_v1', 0),
             (2, 'session_activity_query_fields_v2', 0),
             (3, 'hook_event_retention_index_v3', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        let migrated = conn
            .query_row(
                "SELECT
                    backup.artifact_id,
                    backup.operation_id,
                    backup.provider_id,
                    backup.provider_session_id,
                    backup.source_path,
                    artifact.artifact_kind,
                    artifact.path,
                    artifact.content_hash,
                    artifact.byte_size,
                    artifact.storage_kind
                 FROM backups backup
                 JOIN artifact_manifests artifact ON artifact.id = backup.artifact_id
                 WHERE backup.id = 'backup-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .unwrap();
        let legacy_backup_path_column: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('backups') WHERE name = 'backup_path'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let foreign_key_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(migrated.0, "legacy-backup-artifact:backup-1");
        assert_eq!(migrated.1.as_deref(), Some("operation-1"));
        assert_eq!(migrated.2.as_deref(), Some("codex"));
        assert_eq!(migrated.3, None);
        assert_eq!(migrated.4, "/source/session.jsonl");
        assert_eq!(migrated.5, "session_backup");
        assert_eq!(migrated.6, "/backup/session.jsonl");
        assert_eq!(migrated.7, "legacy-hash");
        assert_eq!(migrated.8, 42);
        assert_eq!(migrated.9, "unknown");
        assert!(!legacy_backup_path_column);
        assert_eq!(foreign_key_violations, 0);
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v4_backups_to_optional_source_paths() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        conn.execute_batch(V3_SCHEMA).unwrap();
        conn.execute_batch(V4_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO artifact_manifests
             (id, artifact_kind, operation_id, provider_id, provider_session_id,
              storage_kind, path, content_hash, byte_size, created_at_ms, metadata_json)
             VALUES
             ('artifact-1', 'session_backup', 'operation-1', 'codex', 'provider-session-1',
              'file', '/backup/session.json', 'sha256:test', 42, 1000, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO backups
             (id, artifact_id, operation_id, provider_id, provider_session_id, session_id,
              source_path, created_at_ms, restore_hint, metadata_json)
             VALUES
             ('backup-1', 'artifact-1', 'operation-1', 'codex', 'provider-session-1',
              NULL, '/source/session.jsonl', 1000, 'import canonical JSON', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES
             (1, 'local_session_store_v1', 0),
             (2, 'session_activity_query_fields_v2', 0),
             (3, 'hook_event_retention_index_v3', 0),
             (4, 'artifact_manifest_store_v4', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        let source_path_not_null: i64 = conn
            .query_row(
                "SELECT \"notnull\"
                 FROM pragma_table_info('backups')
                 WHERE name = 'source_path'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let migrated_source_path: Option<String> = conn
            .query_row(
                "SELECT source_path FROM backups WHERE id = 'backup-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let foreign_key_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(source_path_not_null, 0);
        assert_eq!(
            migrated_source_path.as_deref(),
            Some("/source/session.jsonl")
        );
        assert_eq!(foreign_key_violations, 0);
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v5_schema_to_backup_restore_attempts() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        conn.execute_batch(V3_SCHEMA).unwrap();
        conn.execute_batch(V4_SCHEMA).unwrap();
        conn.execute_batch(V5_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES
             (1, 'local_session_store_v1', 0),
             (2, 'session_activity_query_fields_v2', 0),
             (3, 'hook_event_retention_index_v3', 0),
             (4, 'artifact_manifest_store_v4', 0),
             (5, 'optional_backup_source_path_v5', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        assert!(table_exists(&conn, "backup_restores"));
        let restore_indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM sqlite_master
                 WHERE type = 'index'
                   AND name IN (
                       'idx_backup_restores_backup_started',
                       'idx_backup_restores_status_started'
                   )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(restore_indexes, 2);
        assert_eq!(migration_count, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_existing_v6_event_index_rows_to_turn_links() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        create_schema_migrations_table(&conn).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        conn.execute_batch(V3_SCHEMA).unwrap();
        conn.execute_batch(V4_SCHEMA).unwrap();
        conn.execute_batch(V5_SCHEMA).unwrap();
        conn.execute_batch(V6_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO session_event_index
             (provider_id, session_id, source_path, file_mtime_ms, file_size_bytes,
              event_index, byte_offset, byte_length, line_no)
             VALUES ('codex', 'session-1', '/tmp/session.jsonl', 10, 20, 0, 0, 12, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at_ms)
             VALUES
             (1, 'local_session_store_v1', 0),
             (2, 'session_activity_query_fields_v2', 0),
             (3, 'hook_event_retention_index_v3', 0),
             (4, 'artifact_manifest_store_v4', 0),
             (5, 'optional_backup_source_path_v5', 0),
             (6, 'backup_restore_attempts_v6', 0)",
            [],
        )
        .unwrap();

        apply_schema(&mut conn).unwrap();

        let migrated: (String, i64, Option<String>, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT session_id, line_no, provider_turn_id, turn_index, turn_boundary
                 FROM session_event_index",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        let turn_index_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'index' AND name = 'idx_session_event_index_turn'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(migrated.0, "session-1");
        assert_eq!(migrated.1, 1);
        assert_eq!(migrated.2, None);
        assert_eq!(migrated.3, None);
        assert_eq!(migrated.4, None);
        assert!(turn_index_exists);
    }

    #[test]
    fn initialization_does_not_create_legacy_json_state_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".memorph").join("memorph.db");

        drop(LocalSqliteStore::open(&path).unwrap());

        assert!(path.exists());
        assert!(!dir.path().join(".memorph/session_state.json").exists());
        assert!(!dir.path().join(".memorph/session_overrides.json").exists());
        assert!(!dir.path().join(".memorph/sync").exists());
        assert!(!dir.path().join(".memorph/hooks/events.jsonl").exists());
    }
}
