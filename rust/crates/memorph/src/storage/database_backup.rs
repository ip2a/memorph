use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{backup::Backup, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

use super::{atomic_write, local_store};
use crate::storage::activity_store::ActivityActor;

pub const DATABASE_BACKUP_FORMAT: &str = "memorph-database-backup-v1";
pub const DATABASE_BACKUP_MIME_TYPE: &str = "application/vnd.memorph.database-backup";
const DATABASE_FILE_NAME: &str = "memorph.db";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseBackupPurpose {
    Manual,
    PreRestore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseBackupManifest {
    pub manifest_version: u32,
    pub format: String,
    pub backup_id: String,
    pub purpose: DatabaseBackupPurpose,
    pub actor: ActivityActor,
    pub created_at_ms: i64,
    pub application_version: String,
    pub schema_version: i64,
    pub source_path: PathBuf,
    pub database_file: String,
    pub database_hash: String,
    pub database_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedDatabaseBackup {
    pub bundle_path: PathBuf,
    pub database_path: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: DatabaseBackupManifest,
    pub quick_check: String,
    pub foreign_key_violations: usize,
}

pub fn default_database_backup_root() -> Result<PathBuf> {
    Ok(crate::config::memorph_dir()?
        .join("artifacts")
        .join("database-backups"))
}

pub fn create_database_backup_bundle(
    source: &Connection,
    source_path: &Path,
    output_root: Option<&Path>,
    purpose: DatabaseBackupPurpose,
    actor: ActivityActor,
) -> Result<VerifiedDatabaseBackup> {
    let output_root = output_root
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(default_database_backup_root)?;
    std::fs::create_dir_all(&output_root).with_context(|| {
        format!(
            "Failed to create database backup root: {}",
            output_root.display()
        )
    })?;

    let backup_id = Uuid::new_v4().to_string();
    let created_at_ms = Utc::now().timestamp_millis();
    let bundle_path = output_root.join(format!("{created_at_ms}-{backup_id}"));
    std::fs::create_dir(&bundle_path).with_context(|| {
        format!(
            "Failed to create database backup bundle: {}",
            bundle_path.display()
        )
    })?;

    let result = create_database_backup_bundle_in(
        source,
        source_path,
        &bundle_path,
        backup_id,
        created_at_ms,
        purpose,
        actor,
    );
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&bundle_path);
    }
    result
}

fn create_database_backup_bundle_in(
    source: &Connection,
    source_path: &Path,
    bundle_path: &Path,
    backup_id: String,
    created_at_ms: i64,
    purpose: DatabaseBackupPurpose,
    actor: ActivityActor,
) -> Result<VerifiedDatabaseBackup> {
    let database_path = bundle_path.join(DATABASE_FILE_NAME);
    let temp_database_path = bundle_path.join(format!(".{DATABASE_FILE_NAME}.tmp"));
    let mut destination = Connection::open(&temp_database_path).with_context(|| {
        format!(
            "Failed to create database backup file: {}",
            temp_database_path.display()
        )
    })?;
    {
        let backup = Backup::new(source, &mut destination)
            .context("Failed to initialize SQLite online backup")?;
        backup
            .run_to_completion(128, Duration::from_millis(10), None)
            .context("Failed to create SQLite online backup")?;
    }
    destination
        .pragma_update(None, "journal_mode", "DELETE")
        .context("Failed to make database backup self-contained")?;
    drop(destination);

    sync_file(&temp_database_path)?;
    std::fs::rename(&temp_database_path, &database_path).with_context(|| {
        format!(
            "Failed to publish database backup file: {}",
            database_path.display()
        )
    })?;

    let database_bytes = std::fs::metadata(&database_path)
        .with_context(|| format!("Failed to inspect backup: {}", database_path.display()))?
        .len();
    let manifest = DatabaseBackupManifest {
        manifest_version: MANIFEST_VERSION,
        format: DATABASE_BACKUP_FORMAT.to_string(),
        backup_id,
        purpose,
        actor,
        created_at_ms,
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: read_schema_version(source)?,
        source_path: source_path.to_path_buf(),
        database_file: DATABASE_FILE_NAME.to_string(),
        database_hash: hash_file(&database_path)?,
        database_bytes,
    };
    let manifest_path = bundle_path.join(MANIFEST_FILE_NAME);
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .context("Failed to encode database backup manifest")?;
    atomic_write::write_string_atomic(&manifest_path, &(manifest_json + "\n"))?;

    verify_database_backup_bundle(bundle_path)
}

pub fn verify_database_backup_bundle(bundle_path: &Path) -> Result<VerifiedDatabaseBackup> {
    reject_symlink(bundle_path, "database backup bundle")?;
    let metadata = std::fs::metadata(bundle_path).with_context(|| {
        format!(
            "Failed to inspect database backup bundle: {}",
            bundle_path.display()
        )
    })?;
    if !metadata.is_dir() {
        bail!(
            "Database backup bundle is not a directory: {}",
            bundle_path.display()
        );
    }
    validate_bundle_entries(bundle_path)?;

    let manifest_path = bundle_path.join(MANIFEST_FILE_NAME);
    let database_path = bundle_path.join(DATABASE_FILE_NAME);
    reject_symlink(&manifest_path, "database backup manifest")?;
    reject_symlink(&database_path, "database backup file")?;
    if !std::fs::metadata(&database_path)
        .with_context(|| format!("Failed to inspect backup: {}", database_path.display()))?
        .is_file()
    {
        bail!(
            "Database backup payload is not a file: {}",
            database_path.display()
        );
    }

    let manifest: DatabaseBackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).with_context(|| {
            format!(
                "Failed to read database backup manifest: {}",
                manifest_path.display()
            )
        })?)
        .context("Failed to decode database backup manifest")?;
    validate_manifest(&manifest, bundle_path)?;

    let actual_bytes = std::fs::metadata(&database_path)?.len();
    if actual_bytes != manifest.database_bytes {
        bail!(
            "Database backup size changed: expected {}, found {}",
            manifest.database_bytes,
            actual_bytes
        );
    }
    let actual_hash = hash_file(&database_path)?;
    if actual_hash != manifest.database_hash {
        bail!(
            "Database backup hash changed: expected {}, found {}",
            manifest.database_hash,
            actual_hash
        );
    }

    let conn = open_backup_database(&database_path)?;
    let schema_version = read_schema_version(&conn)?;
    if schema_version != manifest.schema_version {
        bail!(
            "Database backup schema version mismatch: manifest {}, database {}",
            manifest.schema_version,
            schema_version
        );
    }
    let quick_check = run_quick_check(&conn)?;
    let foreign_key_violations = count_foreign_key_violations(&conn)?;
    if foreign_key_violations != 0 {
        bail!(
            "Database backup has {} foreign key violation(s)",
            foreign_key_violations
        );
    }

    Ok(VerifiedDatabaseBackup {
        bundle_path: bundle_path.to_path_buf(),
        database_path,
        manifest_path,
        manifest,
        quick_check,
        foreign_key_violations,
    })
}

pub fn restore_database_from_backup(
    destination: &mut Connection,
    verified: &VerifiedDatabaseBackup,
) -> Result<()> {
    let source = open_backup_database(&verified.database_path)?;
    {
        let backup = Backup::new(&source, destination)
            .context("Failed to initialize SQLite database restore")?;
        backup
            .run_to_completion(128, Duration::from_millis(10), None)
            .context("Failed to restore SQLite database")?;
    }
    local_store::configure_connection(destination)
        .context("Failed to reconfigure restored memorph database")?;
    local_store::apply_schema(destination)
        .context("Failed to migrate restored memorph database")?;
    validate_database_connection(destination)?;
    Ok(())
}

pub fn validate_database_connection(conn: &Connection) -> Result<()> {
    run_quick_check(conn)?;
    let violations = count_foreign_key_violations(conn)?;
    if violations != 0 {
        bail!("Memorph database has {violations} foreign key violation(s)");
    }
    let schema_version = read_schema_version(conn)?;
    if schema_version != local_store::current_schema_version() {
        bail!(
            "Memorph database schema is not current after restore: expected {}, found {}",
            local_store::current_schema_version(),
            schema_version
        );
    }
    Ok(())
}

fn validate_manifest(manifest: &DatabaseBackupManifest, bundle_path: &Path) -> Result<()> {
    if manifest.manifest_version != MANIFEST_VERSION {
        bail!(
            "Unsupported database backup manifest version: {}",
            manifest.manifest_version
        );
    }
    if manifest.format != DATABASE_BACKUP_FORMAT {
        bail!("Unsupported database backup format: {}", manifest.format);
    }
    if manifest.database_file != DATABASE_FILE_NAME {
        bail!(
            "Database backup manifest points at an unsupported payload: {}",
            manifest.database_file
        );
    }
    if manifest.backup_id.trim().is_empty() {
        bail!("Database backup manifest has no backup identity");
    }
    if manifest.schema_version <= 0 {
        bail!(
            "Database backup has invalid schema version: {}",
            manifest.schema_version
        );
    }
    if manifest.schema_version > local_store::current_schema_version() {
        bail!(
            "Database backup schema {} is newer than supported schema {}",
            manifest.schema_version,
            local_store::current_schema_version()
        );
    }
    if !manifest.database_hash.starts_with("sha256:") {
        bail!("Database backup manifest has an unsupported content hash");
    }
    if manifest.database_bytes == 0 {
        bail!("Database backup manifest records an empty database");
    }
    if bundle_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_none()
    {
        bail!("Database backup bundle path is not valid UTF-8");
    }
    Ok(())
}

fn validate_bundle_entries(bundle_path: &Path) -> Result<()> {
    let mut entries = std::fs::read_dir(bundle_path)
        .with_context(|| format!("Failed to read backup bundle: {}", bundle_path.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .context("Failed to read database backup bundle entry")
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort();
    let mut expected = vec![
        DATABASE_FILE_NAME.to_string(),
        MANIFEST_FILE_NAME.to_string(),
    ];
    expected.sort();
    if entries != expected {
        bail!(
            "Database backup bundle must contain exactly {} and {}",
            DATABASE_FILE_NAME,
            MANIFEST_FILE_NAME
        );
    }
    Ok(())
}

fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {label}: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("{label} must not be a symbolic link: {}", path.display());
    }
    Ok(())
}

fn open_backup_database(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Failed to open database backup: {}", path.display()))
}

fn read_schema_version(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
    .context("Failed to read memorph database schema version")
}

fn run_quick_check(conn: &Connection) -> Result<String> {
    let mut stmt = conn
        .prepare("PRAGMA quick_check")
        .context("Failed to prepare SQLite quick check")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .context("Failed to run SQLite quick check")?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row.context("Failed to decode SQLite quick check result")?);
    }
    if results.as_slice() != ["ok"] {
        bail!("SQLite quick check failed: {}", results.join("; "));
    }
    Ok("ok".to_string())
}

fn count_foreign_key_violations(conn: &Connection) -> Result<usize> {
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .context("Failed to prepare SQLite foreign key check")?;
    let mut rows = stmt
        .query([])
        .context("Failed to run SQLite foreign key check")?;
    let mut count = 0;
    while rows
        .next()
        .context("Failed to decode SQLite foreign key check result")?
        .is_some()
    {
        count += 1;
    }
    Ok(count)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open database backup: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read database backup: {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn sync_file(path: &Path) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("Failed to reopen backup for sync: {}", path.display()))?;
    file.flush()
        .with_context(|| format!("Failed to flush backup: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("Failed to sync backup: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_database(path: &Path) -> Connection {
        let mut conn = Connection::open(path).unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO sync_groups (id, title, source_provider, created_at_ms, updated_at_ms, status) VALUES ('group-1', 'Test', 'claude', 1, 1, 'active')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn creates_and_verifies_self_contained_database_backup() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let conn = source_database(&source_path);

        let verified = create_database_backup_bundle(
            &conn,
            &source_path,
            Some(&dir.path().join("backups")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();

        assert_eq!(verified.quick_check, "ok");
        assert_eq!(verified.foreign_key_violations, 0);
        assert_eq!(
            verified.manifest.schema_version,
            local_store::current_schema_version()
        );
        assert!(verified.database_path.is_file());
        assert!(verified.manifest_path.is_file());
        assert!(!verified.database_path.with_extension("db-wal").exists());
        let backup = open_backup_database(&verified.database_path).unwrap();
        let count: i64 = backup
            .query_row("SELECT COUNT(*) FROM sync_groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn rejects_changed_database_backup_payload() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let conn = source_database(&source_path);
        let verified = create_database_backup_bundle(
            &conn,
            &source_path,
            Some(&dir.path().join("backups")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&verified.database_path)
            .unwrap();
        file.write_all(b"changed").unwrap();

        let error = verify_database_backup_bundle(&verified.bundle_path).unwrap_err();
        assert!(error.to_string().contains("size changed"));
    }

    #[test]
    fn restores_database_and_replaces_management_state() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let source = source_database(&source_path);
        let verified = create_database_backup_bundle(
            &source,
            &source_path,
            Some(&dir.path().join("backups")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();

        let destination_path = dir.path().join("destination.db");
        let mut destination = source_database(&destination_path);
        destination.execute("DELETE FROM sync_groups", []).unwrap();
        restore_database_from_backup(&mut destination, &verified).unwrap();

        let count: i64 = destination
            .query_row("SELECT COUNT(*) FROM sync_groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
