use anyhow::{bail, Context as _, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use walkdir::WalkDir;

use super::activity_store::ActivityActor;

const DEFAULT_QUERY_LIMIT: usize = 100;
const MAX_QUERY_LIMIT: usize = 500;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactManifestKind {
    CompressionArchive,
    DatabaseBackup,
    SessionExport,
    SessionBackup,
}

impl ArtifactManifestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CompressionArchive => "compression_archive",
            Self::DatabaseBackup => "database_backup",
            Self::SessionExport => "session_export",
            Self::SessionBackup => "session_backup",
        }
    }
}

impl fmt::Display for ArtifactManifestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ArtifactManifestKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "compression_archive" => Ok(Self::CompressionArchive),
            "database_backup" => Ok(Self::DatabaseBackup),
            "session_export" => Ok(Self::SessionExport),
            "session_backup" => Ok(Self::SessionBackup),
            _ => bail!("Unknown artifact manifest kind: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStorageKind {
    File,
    Directory,
    Unknown,
}

impl ArtifactStorageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ArtifactStorageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ArtifactStorageKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "file" => Ok(Self::File),
            "directory" => Ok(Self::Directory),
            "unknown" => Ok(Self::Unknown),
            _ => bail!("Unknown artifact storage kind: {value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewArtifactManifest {
    pub artifact_kind: ArtifactManifestKind,
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_id: Option<String>,
    pub projection_report_id: Option<String>,
    pub path: PathBuf,
    pub mime_type: Option<String>,
    pub format: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactManifest {
    pub id: String,
    pub artifact_kind: ArtifactManifestKind,
    pub storage_kind: ArtifactStorageKind,
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_id: Option<String>,
    pub projection_report_id: Option<String>,
    pub path: PathBuf,
    pub content_hash: String,
    pub byte_size: i64,
    pub mime_type: Option<String>,
    pub format: Option<String>,
    pub created_at_ms: i64,
    pub metadata: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ArtifactQuery {
    pub artifact_kind: Option<ArtifactManifestKind>,
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_id: Option<String>,
    pub projection_report_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactVerificationStatus {
    Verified,
    Missing,
    Changed,
    Unverifiable,
}

impl fmt::Display for ArtifactVerificationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Verified => "verified",
            Self::Missing => "missing",
            Self::Changed => "changed",
            Self::Unverifiable => "unverifiable",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactVerification {
    pub artifact_id: String,
    pub path: PathBuf,
    pub status: ArtifactVerificationStatus,
    pub expected_content_hash: String,
    pub actual_content_hash: Option<String>,
    pub expected_byte_size: i64,
    pub actual_byte_size: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRetentionState {
    Retained,
}

impl ArtifactRetentionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retained => "retained",
        }
    }
}

impl fmt::Display for ArtifactRetentionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactInspectionEntry {
    pub manifest: ArtifactManifest,
    pub verification: ArtifactVerification,
    pub retention_state: ArtifactRetentionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrphanArtifactFile {
    pub path: PathBuf,
    pub byte_size: i64,
    pub modified_at_ms: i64,
    pub managed_layout: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactInspectionReport {
    pub generated_at_ms: i64,
    pub managed_blob_root: PathBuf,
    pub registered: Vec<ArtifactInspectionEntry>,
    pub orphan_files: Vec<OrphanArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactCleanupFailure {
    pub path: Option<PathBuf>,
    pub artifact_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactCleanupReport {
    pub applied: bool,
    pub cutoff_ms: i64,
    pub candidate_orphan_paths: Vec<PathBuf>,
    pub deleted_paths: Vec<PathBuf>,
    pub retained_shared_paths: Vec<PathBuf>,
    pub failures: Vec<ArtifactCleanupFailure>,
}

#[derive(Debug, Clone)]
pub struct NewBackupRecord {
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_id: Option<String>,
    pub source_path: Option<PathBuf>,
    pub backup_path: PathBuf,
    pub restore_hint: Option<String>,
    pub mime_type: Option<String>,
    pub format: Option<String>,
    pub artifact_metadata: Value,
    pub backup_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupRecord {
    pub id: String,
    pub artifact: ArtifactManifest,
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_id: Option<String>,
    pub source_path: Option<PathBuf>,
    pub created_at_ms: i64,
    pub restore_hint: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupRestoreStatus {
    Running,
    Success,
    Failed,
}

impl BackupRestoreStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for BackupRestoreStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BackupRestoreStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            _ => bail!("Unknown backup restore status: {value}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupRestoreRecord {
    pub id: String,
    pub backup_id: String,
    pub status: BackupRestoreStatus,
    pub actor: ActivityActor,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BackupQuery {
    pub operation_id: Option<String>,
    pub provider_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub restore_status: Option<BackupRestoreStatus>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupEntry {
    pub backup: BackupRecord,
    pub latest_restore: Option<BackupRestoreRecord>,
}

pub struct ArtifactStore<'a> {
    conn: &'a mut Connection,
}

impl<'a> ArtifactStore<'a> {
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    pub fn register_path(&mut self, manifest: NewArtifactManifest) -> Result<ArtifactManifest> {
        let mut stored = self.register_paths(vec![manifest])?;
        Ok(stored.remove(0))
    }

    pub fn register_paths(
        &mut self,
        manifests: Vec<NewArtifactManifest>,
    ) -> Result<Vec<ArtifactManifest>> {
        if manifests.is_empty() {
            return Ok(Vec::new());
        }
        let inspected = manifests
            .iter()
            .map(|manifest| inspect_artifact_path(&manifest.path))
            .collect::<Result<Vec<_>>>()?;
        let tx = self
            .conn
            .transaction()
            .context("Failed to start artifact registration transaction")?;
        let mut stored = Vec::with_capacity(manifests.len());
        for (manifest, inspected) in manifests.into_iter().zip(inspected) {
            let links = resolve_artifact_links(&tx, &manifest)?;
            stored.push(insert_artifact_manifest(&tx, manifest, links, inspected)?);
        }
        tx.commit()
            .context("Failed to commit artifact registration transaction")?;
        Ok(stored)
    }

    pub fn register_backup(&mut self, backup: NewBackupRecord) -> Result<BackupRecord> {
        let source_path = backup
            .source_path
            .as_deref()
            .map(|path| {
                std::fs::canonicalize(path).with_context(|| {
                    format!("Failed to resolve backup source path: {}", path.display())
                })
            })
            .transpose()?;
        let inspected = inspect_artifact_path(&backup.backup_path)?;
        let artifact_input = NewArtifactManifest {
            artifact_kind: ArtifactManifestKind::SessionBackup,
            operation_id: backup.operation_id.clone(),
            provider_id: backup.provider_id.clone(),
            provider_session_id: backup.provider_session_id.clone(),
            session_id: backup.session_id.clone(),
            projection_report_id: None,
            path: backup.backup_path.clone(),
            mime_type: backup.mime_type.clone(),
            format: backup.format.clone(),
            metadata: backup.artifact_metadata.clone(),
        };
        let links = resolve_artifact_links(self.conn, &artifact_input)?;
        let tx = self
            .conn
            .transaction()
            .context("Failed to start backup registration transaction")?;
        let artifact = insert_artifact_manifest(&tx, artifact_input, links, inspected)?;
        let record = insert_backup_record(&tx, backup, source_path, artifact)?;
        tx.commit()
            .context("Failed to commit backup registration transaction")?;
        Ok(record)
    }

    pub fn get(&self, artifact_id: &str) -> Result<Option<ArtifactManifest>> {
        load_artifact_by_id(self.conn, artifact_id)
    }

    pub fn get_backup(&self, backup_id: &str) -> Result<Option<BackupRecord>> {
        load_backup_by_id(self.conn, backup_id)
    }

    pub fn get_backup_entry(&self, backup_id: &str) -> Result<Option<BackupEntry>> {
        let Some(backup) = self.get_backup(backup_id)? else {
            return Ok(None);
        };
        Ok(Some(BackupEntry {
            latest_restore: load_latest_backup_restore(self.conn, backup_id)?,
            backup,
        }))
    }

    pub fn query_backups(&self, query: BackupQuery) -> Result<Vec<BackupEntry>> {
        let limit = query
            .limit
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .clamp(1, MAX_QUERY_LIMIT) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT backup.id
                 FROM backups backup
                 LEFT JOIN backup_restores latest_restore
                   ON latest_restore.id = (
                       SELECT restore.id
                       FROM backup_restores restore
                       WHERE restore.backup_id = backup.id
                       ORDER BY restore.started_at_ms DESC, restore.id DESC
                       LIMIT 1
                   )
                 WHERE (?1 IS NULL OR backup.operation_id = ?1)
                   AND (?2 IS NULL OR backup.provider_id = ?2)
                   AND (?3 IS NULL OR backup.provider_session_id = ?3)
                   AND (?4 IS NULL OR latest_restore.status = ?4)
                 ORDER BY backup.created_at_ms DESC, backup.id DESC
                 LIMIT ?5",
            )
            .context("Failed to prepare backup query")?;
        let rows = stmt
            .query_map(
                params![
                    query.operation_id,
                    query.provider_id,
                    query.provider_session_id,
                    query.restore_status.map(BackupRestoreStatus::as_str),
                    limit,
                ],
                |row| row.get::<_, String>(0),
            )
            .context("Failed to query backups")?;
        let mut entries = Vec::new();
        for row in rows {
            let backup_id = row.context("Failed to decode backup query row")?;
            entries.push(
                self.get_backup_entry(&backup_id)?
                    .context("Backup disappeared while loading query results")?,
            );
        }
        Ok(entries)
    }

    pub fn start_backup_restore(
        &mut self,
        backup_id: &str,
        actor: ActivityActor,
    ) -> Result<BackupRestoreRecord> {
        if self.get_backup(backup_id)?.is_none() {
            bail!("Unknown backup: {backup_id}");
        }
        let record = BackupRestoreRecord {
            id: Uuid::new_v4().to_string(),
            backup_id: backup_id.to_string(),
            status: BackupRestoreStatus::Running,
            actor,
            started_at_ms: Utc::now().timestamp_millis(),
            finished_at_ms: None,
            error: None,
        };
        self.conn
            .execute(
                "INSERT INTO backup_restores
                 (id, backup_id, status, actor, started_at_ms, finished_at_ms, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
                params![
                    record.id,
                    record.backup_id,
                    record.status.as_str(),
                    record.actor.as_str(),
                    record.started_at_ms,
                ],
            )
            .context("Failed to start backup restore record")?;
        Ok(record)
    }

    pub fn finish_backup_restore(
        &mut self,
        restore_id: &str,
        status: BackupRestoreStatus,
        error: Option<&str>,
    ) -> Result<BackupRestoreRecord> {
        if status == BackupRestoreStatus::Running {
            bail!("Backup restore completion status must be terminal");
        }
        let finished_at_ms = Utc::now().timestamp_millis();
        let updated = self
            .conn
            .execute(
                "UPDATE backup_restores
                 SET status = ?1, finished_at_ms = ?2, error = ?3
                 WHERE id = ?4 AND status = 'running'",
                params![status.as_str(), finished_at_ms, error, restore_id],
            )
            .context("Failed to finish backup restore record")?;
        if updated != 1 {
            bail!("Backup restore is missing or already finished: {restore_id}");
        }
        load_backup_restore(self.conn, restore_id)?
            .context("Finished backup restore record disappeared")
    }

    pub fn find_backup_by_artifact_path(&self, path: &Path) -> Result<Option<BackupRecord>> {
        let path = std::fs::canonicalize(path).with_context(|| {
            format!("Failed to resolve backup artifact path: {}", path.display())
        })?;
        let path_text = path.to_string_lossy().to_string();
        let backup_id = self
            .conn
            .query_row(
                "SELECT backup.id
                 FROM backups backup
                 JOIN artifact_manifests artifact ON artifact.id = backup.artifact_id
                 WHERE artifact.path = ?1
                 ORDER BY backup.created_at_ms DESC, backup.id
                 LIMIT 1",
                [path_text],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("Failed to find backup by artifact path")?;
        backup_id
            .as_deref()
            .map(|backup_id| load_backup_by_id(self.conn, backup_id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn delete_backup_metadata(&mut self, backup_id: &str) -> Result<bool> {
        let tx = self
            .conn
            .transaction()
            .context("Failed to start backup deletion transaction")?;
        let artifact_id = tx
            .query_row(
                "SELECT artifact_id FROM backups WHERE id = ?1",
                [backup_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("Failed to load backup artifact for deletion")?;
        let Some(artifact_id) = artifact_id else {
            tx.commit()
                .context("Failed to commit empty backup deletion transaction")?;
            return Ok(false);
        };
        tx.execute("DELETE FROM backups WHERE id = ?1", [backup_id])
            .context("Failed to delete backup record")?;
        tx.execute(
            "DELETE FROM artifact_manifests WHERE id = ?1",
            [artifact_id],
        )
        .context("Failed to delete backup artifact manifest")?;
        tx.commit()
            .context("Failed to commit backup deletion transaction")?;
        Ok(true)
    }

    pub fn query(&self, query: ArtifactQuery) -> Result<Vec<ArtifactManifest>> {
        let limit = query
            .limit
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .clamp(1, MAX_QUERY_LIMIT) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    id, artifact_kind, storage_kind, operation_id, provider_id,
                    provider_session_id, session_id, projection_report_id,
                    path, content_hash, byte_size, mime_type, format, created_at_ms, metadata_json
                 FROM artifact_manifests
                 WHERE (?1 IS NULL OR artifact_kind = ?1)
                   AND (?2 IS NULL OR operation_id = ?2)
                   AND (?3 IS NULL OR provider_id = ?3)
                   AND (?4 IS NULL OR provider_session_id = ?4)
                   AND (?5 IS NULL OR session_id = ?5)
                   AND (?6 IS NULL OR projection_report_id = ?6)
                 ORDER BY created_at_ms DESC, id DESC
                 LIMIT ?7",
            )
            .context("Failed to prepare artifact manifest query")?;
        let rows = stmt
            .query_map(
                params![
                    query.artifact_kind.map(|value| value.as_str()),
                    query.operation_id,
                    query.provider_id,
                    query.provider_session_id,
                    query.session_id,
                    query.projection_report_id,
                    limit
                ],
                decode_artifact_row,
            )
            .context("Failed to query artifact manifests")?;
        let mut manifests = Vec::new();
        for row in rows {
            manifests.push(row.context("Failed to decode artifact manifest")?);
        }
        Ok(manifests)
    }

    pub fn verify(&self, artifact_id: &str) -> Result<Option<ArtifactVerification>> {
        let Some(manifest) = self.get(artifact_id)? else {
            return Ok(None);
        };
        Ok(Some(verify_artifact_manifest(&manifest)?))
    }

    pub fn inspect(&self, managed_blob_root: &Path) -> Result<ArtifactInspectionReport> {
        let manifests = load_all_artifact_manifests(self.conn)?;
        let registered_paths = manifests
            .iter()
            .map(|manifest| artifact_path_for_comparison(&manifest.path))
            .collect::<Result<HashSet<_>>>()?;
        let mut registered = Vec::with_capacity(manifests.len());
        for manifest in manifests {
            let verification = verify_artifact_manifest(&manifest)?;
            registered.push(ArtifactInspectionEntry {
                manifest,
                verification,
                retention_state: ArtifactRetentionState::Retained,
            });
        }
        let orphan_files = scan_orphan_artifact_files(managed_blob_root, &registered_paths)?;
        Ok(ArtifactInspectionReport {
            generated_at_ms: Utc::now().timestamp_millis(),
            managed_blob_root: managed_blob_root.to_path_buf(),
            registered,
            orphan_files,
        })
    }

    pub fn cleanup_orphan_files(
        &mut self,
        managed_blob_root: &Path,
        cutoff_ms: i64,
        apply: bool,
    ) -> Result<ArtifactCleanupReport> {
        let registered_paths = load_all_artifact_paths(self.conn)?;
        let orphan_files = scan_orphan_artifact_files(managed_blob_root, &registered_paths)?
            .into_iter()
            .filter(|file| file.managed_layout && file.modified_at_ms < cutoff_ms)
            .collect::<Vec<_>>();
        let mut report = ArtifactCleanupReport {
            applied: apply,
            cutoff_ms,
            candidate_orphan_paths: orphan_files.iter().map(|file| file.path.clone()).collect(),
            deleted_paths: Vec::new(),
            retained_shared_paths: Vec::new(),
            failures: Vec::new(),
        };
        if !apply {
            sort_cleanup_report(&mut report);
            return Ok(report);
        }

        let mut paths_to_delete = Vec::new();
        for orphan in orphan_files {
            let path_text = orphan.path.to_string_lossy().to_string();
            if load_artifact_ids_by_path(self.conn, &path_text)?.is_empty() {
                paths_to_delete.push(orphan.path);
            } else {
                report.failures.push(ArtifactCleanupFailure {
                    path: Some(orphan.path),
                    artifact_ids: Vec::new(),
                    reason: "Orphan file became registered before cleanup".to_string(),
                });
            }
        }

        paths_to_delete.sort();
        paths_to_delete.dedup();
        for path in paths_to_delete {
            let path_text = path.to_string_lossy().to_string();
            if !load_artifact_ids_by_path(self.conn, &path_text)?.is_empty() {
                report.retained_shared_paths.push(path);
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => report.deleted_paths.push(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => report.failures.push(ArtifactCleanupFailure {
                    path: Some(path),
                    artifact_ids: Vec::new(),
                    reason: format!("Failed to delete orphan artifact file: {error}"),
                }),
            }
        }

        sort_cleanup_report(&mut report);
        Ok(report)
    }
}

fn load_all_artifact_manifests(conn: &Connection) -> Result<Vec<ArtifactManifest>> {
    let mut stmt = conn
        .prepare(
            "SELECT
                id, artifact_kind, storage_kind, operation_id, provider_id,
                provider_session_id, session_id, projection_report_id,
                path, content_hash, byte_size, mime_type, format, created_at_ms, metadata_json
             FROM artifact_manifests
             ORDER BY created_at_ms DESC, id DESC",
        )
        .context("Failed to prepare complete artifact inspection query")?;
    let rows = stmt
        .query_map([], decode_artifact_row)
        .context("Failed to query complete artifact inspection")?;
    let mut manifests = Vec::new();
    for row in rows {
        manifests.push(row.context("Failed to decode artifact manifest")?);
    }
    Ok(manifests)
}

fn sort_cleanup_report(report: &mut ArtifactCleanupReport) {
    report.candidate_orphan_paths.sort();
    report.deleted_paths.sort();
    report.retained_shared_paths.sort();
    report.failures.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.artifact_ids.cmp(&right.artifact_ids))
            .then_with(|| left.reason.cmp(&right.reason))
    });
}

fn artifact_path_for_comparison(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        std::fs::canonicalize(path)
            .with_context(|| format!("Failed to resolve artifact path: {}", path.display()))
    } else {
        Ok(path.to_path_buf())
    }
}

fn verify_artifact_manifest(manifest: &ArtifactManifest) -> Result<ArtifactVerification> {
    if !manifest.path.exists() {
        return Ok(ArtifactVerification {
            artifact_id: manifest.id.clone(),
            path: manifest.path.clone(),
            status: ArtifactVerificationStatus::Missing,
            expected_content_hash: manifest.content_hash.clone(),
            actual_content_hash: None,
            expected_byte_size: manifest.byte_size,
            actual_byte_size: None,
        });
    }
    if manifest.storage_kind == ArtifactStorageKind::Unknown
        || !is_supported_content_hash(&manifest.content_hash)
    {
        return Ok(ArtifactVerification {
            artifact_id: manifest.id.clone(),
            path: manifest.path.clone(),
            status: ArtifactVerificationStatus::Unverifiable,
            expected_content_hash: manifest.content_hash.clone(),
            actual_content_hash: None,
            expected_byte_size: manifest.byte_size,
            actual_byte_size: None,
        });
    }

    let inspected = inspect_artifact_path(&manifest.path)?;
    let verified = manifest.storage_kind == inspected.storage_kind
        && manifest.content_hash == inspected.content_hash
        && manifest.byte_size == inspected.byte_size;
    Ok(ArtifactVerification {
        artifact_id: manifest.id.clone(),
        path: manifest.path.clone(),
        status: if verified {
            ArtifactVerificationStatus::Verified
        } else {
            ArtifactVerificationStatus::Changed
        },
        expected_content_hash: manifest.content_hash.clone(),
        actual_content_hash: Some(inspected.content_hash),
        expected_byte_size: manifest.byte_size,
        actual_byte_size: Some(inspected.byte_size),
    })
}

pub(crate) fn default_managed_artifact_root() -> Result<PathBuf> {
    Ok(crate::config::memorph_dir()?
        .join("artifacts")
        .join("blobs")
        .join("sha256"))
}

fn load_all_artifact_paths(conn: &Connection) -> Result<HashSet<PathBuf>> {
    let mut stmt = conn
        .prepare("SELECT path FROM artifact_manifests")
        .context("Failed to prepare artifact path query")?;
    let rows = stmt
        .query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))
        .context("Failed to query artifact paths")?;
    let mut paths = HashSet::new();
    for row in rows {
        let path = row.context("Failed to decode artifact path")?;
        paths.insert(artifact_path_for_comparison(&path)?);
    }
    Ok(paths)
}

fn load_artifact_ids_by_path(conn: &Connection, path: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT id FROM artifact_manifests WHERE path = ?1 ORDER BY id")
        .context("Failed to prepare artifact path reference query")?;
    let rows = stmt
        .query_map([path], |row| row.get::<_, String>(0))
        .context("Failed to query artifact path references")?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.context("Failed to decode artifact path reference")?);
    }
    Ok(ids)
}

fn scan_orphan_artifact_files(
    managed_blob_root: &Path,
    registered_paths: &HashSet<PathBuf>,
) -> Result<Vec<OrphanArtifactFile>> {
    if !managed_blob_root.exists() {
        return Ok(Vec::new());
    }
    let root = canonical_root_for_comparison(managed_blob_root)?;
    let mut files = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry.context("Failed to scan managed artifact directory")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        if registered_paths.contains(&path) {
            continue;
        }
        let metadata = entry
            .metadata()
            .with_context(|| format!("Failed to inspect orphan artifact: {}", path.display()))?;
        let byte_size = i64::try_from(metadata.len())
            .context("Orphan artifact exceeds SQLite integer range")?;
        let modified_at_ms = system_time_ms(metadata.modified().with_context(|| {
            format!(
                "Failed to read orphan artifact modification time: {}",
                path.display()
            )
        })?)?;
        files.push(OrphanArtifactFile {
            managed_layout: is_managed_blob_path(&root, &path),
            path,
            byte_size,
            modified_at_ms,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn canonical_root_for_comparison(root: &Path) -> Result<PathBuf> {
    if root.exists() {
        std::fs::canonicalize(root)
            .with_context(|| format!("Failed to resolve artifact root: {}", root.display()))
    } else {
        Ok(root.to_path_buf())
    }
}

fn is_managed_blob_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.len() != 4 {
        return false;
    }
    let hash = &components[2];
    components[0].len() == 2
        && components[1].len() == 2
        && hash.len() == 64
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && components[0] == hash[..2]
        && components[1] == hash[2..4]
        && components[3].ends_with(".json")
}

fn system_time_ms(time: SystemTime) -> Result<i64> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .context("Artifact modification time predates Unix epoch")?;
    i64::try_from(duration.as_millis()).context("Artifact modification time exceeds i64")
}

#[derive(Debug)]
struct InspectedArtifact {
    path: PathBuf,
    storage_kind: ArtifactStorageKind,
    content_hash: String,
    byte_size: i64,
}

#[derive(Debug)]
struct ResolvedArtifactLinks {
    provider_id: Option<String>,
    provider_session_id: Option<String>,
    session_id: Option<String>,
    projection_report_id: Option<String>,
}

fn inspect_artifact_path(path: &Path) -> Result<InspectedArtifact> {
    let path = std::fs::canonicalize(path)
        .with_context(|| format!("Failed to resolve artifact path: {}", path.display()))?;
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("Failed to read artifact metadata: {}", path.display()))?;
    if metadata.is_file() {
        let (content_hash, byte_size) = hash_file(&path)?;
        return Ok(InspectedArtifact {
            path,
            storage_kind: ArtifactStorageKind::File,
            content_hash,
            byte_size,
        });
    }
    if metadata.is_dir() {
        let (content_hash, byte_size) = hash_directory(&path)?;
        return Ok(InspectedArtifact {
            path,
            storage_kind: ArtifactStorageKind::Directory,
            content_hash,
            byte_size,
        });
    }
    bail!(
        "Artifact path is neither a file nor a directory: {}",
        path.display()
    )
}

fn is_supported_content_hash(content_hash: &str) -> bool {
    content_hash.starts_with("sha256:") || content_hash.starts_with("sha256-tree-v1:")
}

fn hash_file(path: &Path) -> Result<(String, i64)> {
    let file =
        File::open(path).with_context(|| format!("Failed to open artifact: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut byte_size = 0_i64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("Failed to hash artifact: {}", path.display()))?;
        if read == 0 {
            break;
        }
        byte_size = byte_size
            .checked_add(read as i64)
            .context("Artifact size exceeds SQLite integer range")?;
        hasher.update(&buffer[..read]);
    }
    Ok((format!("sha256:{:x}", hasher.finalize()), byte_size))
}

fn hash_directory(path: &Path) -> Result<(String, i64)> {
    let mut hasher = Sha256::new();
    hasher.update(b"memorph-directory-sha256-v1\0");
    let mut byte_size = 0_i64;

    for entry in WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .skip(1)
    {
        let entry = entry
            .with_context(|| format!("Failed to walk artifact directory: {}", path.display()))?;
        let relative = entry.path().strip_prefix(path).with_context(|| {
            format!(
                "Failed to derive artifact directory entry path: {}",
                entry.path().display()
            )
        })?;
        let file_type = entry.file_type();
        if file_type.is_dir() {
            hasher.update(b"D\0");
            hash_relative_path(&mut hasher, relative);
            hasher.update(b"\0");
            continue;
        }
        if file_type.is_symlink() {
            let target = std::fs::read_link(entry.path()).with_context(|| {
                format!(
                    "Failed to read artifact directory symlink: {}",
                    entry.path().display()
                )
            })?;
            hasher.update(b"L\0");
            hash_relative_path(&mut hasher, relative);
            hasher.update(b"\0");
            hasher.update(target.as_os_str().as_encoded_bytes());
            hasher.update(b"\0");
            continue;
        }
        if !file_type.is_file() {
            bail!(
                "Artifact directory contains unsupported entry: {}",
                entry.path().display()
            );
        }

        let metadata = entry.metadata().with_context(|| {
            format!(
                "Failed to read artifact directory entry metadata: {}",
                entry.path().display()
            )
        })?;
        let entry_size = i64::try_from(metadata.len())
            .context("Artifact directory entry size exceeds SQLite integer range")?;
        byte_size = byte_size
            .checked_add(entry_size)
            .context("Artifact directory size exceeds SQLite integer range")?;
        hasher.update(b"F\0");
        hash_relative_path(&mut hasher, relative);
        hasher.update(b"\0");
        hasher.update(metadata.len().to_le_bytes());
        let file = File::open(entry.path()).with_context(|| {
            format!(
                "Failed to open artifact directory entry: {}",
                entry.path().display()
            )
        })?;
        let mut reader = BufReader::new(file);
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buffer).with_context(|| {
                format!(
                    "Failed to hash artifact directory entry: {}",
                    entry.path().display()
                )
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }

    Ok((format!("sha256-tree-v1:{:x}", hasher.finalize()), byte_size))
}

fn hash_relative_path(hasher: &mut Sha256, path: &Path) {
    for component in path.components() {
        hasher.update(component.as_os_str().as_encoded_bytes());
        hasher.update(b"/");
    }
}

fn resolve_artifact_links(
    conn: &Connection,
    manifest: &NewArtifactManifest,
) -> Result<ResolvedArtifactLinks> {
    let mut provider_id = manifest.provider_id.clone();
    let mut provider_session_id = manifest.provider_session_id.clone();
    let mut session_id = manifest.session_id.clone();

    if let Some(report_id) = manifest.projection_report_id.as_deref() {
        let (report_session_id, report_provider_id) = conn
            .query_row(
                "SELECT session_id, provider_id FROM projection_reports WHERE id = ?1",
                [report_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .context("Failed to resolve artifact projection report link")?
            .with_context(|| format!("Artifact projection report does not exist: {report_id}"))?;
        merge_link("session", &mut session_id, report_session_id)?;
        merge_link("provider", &mut provider_id, Some(report_provider_id))?;
    }

    if session_id.is_none() {
        if let (Some(provider_id_value), Some(provider_session_id_value)) =
            (provider_id.as_deref(), provider_session_id.as_deref())
        {
            session_id = conn
                .query_row(
                    "SELECT id
                     FROM sessions
                     WHERE provider_id = ?1 AND provider_session_id = ?2
                     ORDER BY updated_at_ms DESC, id
                     LIMIT 1",
                    params![provider_id_value, provider_session_id_value],
                    |row| row.get(0),
                )
                .optional()
                .context("Failed to resolve artifact canonical session identity")?;
        }
    }

    if let Some(session_id_value) = session_id.as_deref() {
        let (session_provider_id, session_provider_session_id) = conn
            .query_row(
                "SELECT provider_id, provider_session_id FROM sessions WHERE id = ?1",
                [session_id_value],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .context("Failed to resolve artifact session link")?
            .with_context(|| format!("Artifact session does not exist: {session_id_value}"))?;
        merge_link("provider", &mut provider_id, Some(session_provider_id))?;
        merge_link(
            "provider session",
            &mut provider_session_id,
            session_provider_session_id,
        )?;
    }

    Ok(ResolvedArtifactLinks {
        provider_id,
        provider_session_id,
        session_id,
        projection_report_id: manifest.projection_report_id.clone(),
    })
}

fn merge_link(label: &str, current: &mut Option<String>, derived: Option<String>) -> Result<()> {
    let Some(derived) = derived else {
        return Ok(());
    };
    match current {
        Some(value) if value != &derived => {
            bail!("Artifact {label} link conflicts with related SQLite records")
        }
        Some(_) => Ok(()),
        None => {
            *current = Some(derived);
            Ok(())
        }
    }
}

fn insert_artifact_manifest(
    conn: &Transaction<'_>,
    manifest: NewArtifactManifest,
    links: ResolvedArtifactLinks,
    inspected: InspectedArtifact,
) -> Result<ArtifactManifest> {
    let id = Uuid::new_v4().to_string();
    let created_at_ms = Utc::now().timestamp_millis();
    let metadata_json =
        serde_json::to_string(&manifest.metadata).context("Failed to encode artifact metadata")?;
    let path_text = inspected.path.to_string_lossy().to_string();
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO artifact_manifests
             (id, artifact_kind, session_id, path, content_hash, byte_size,
              mime_type, format, created_at_ms, metadata_json, operation_id, provider_id,
              provider_session_id, projection_report_id, storage_kind)
             VALUES
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                manifest.artifact_kind.as_str(),
                links.session_id,
                path_text,
                inspected.content_hash,
                inspected.byte_size,
                manifest.mime_type,
                manifest.format,
                created_at_ms,
                metadata_json,
                manifest.operation_id,
                links.provider_id,
                links.provider_session_id,
                links.projection_report_id,
                inspected.storage_kind.as_str()
            ],
        )
        .context("Failed to insert artifact manifest")?;

    let stored = if inserted == 1 {
        load_artifact_by_id(conn, &id)?
    } else {
        load_artifact_by_registration(
            conn,
            manifest.artifact_kind,
            &path_text,
            &inspected.content_hash,
            manifest.operation_id.as_deref(),
        )?
    }
    .context("Artifact registration did not produce a manifest")?;

    let expected = ArtifactManifest {
        id: stored.id.clone(),
        artifact_kind: manifest.artifact_kind,
        storage_kind: inspected.storage_kind,
        operation_id: manifest.operation_id,
        provider_id: links.provider_id,
        provider_session_id: links.provider_session_id,
        session_id: links.session_id,
        projection_report_id: links.projection_report_id,
        path: inspected.path,
        content_hash: inspected.content_hash,
        byte_size: inspected.byte_size,
        mime_type: manifest.mime_type,
        format: manifest.format,
        created_at_ms: stored.created_at_ms,
        metadata: manifest.metadata,
    };
    if stored != expected {
        bail!(
            "Artifact path was already registered with conflicting context: {}",
            stored.path.display()
        );
    }
    Ok(stored)
}

fn insert_backup_record(
    conn: &Transaction<'_>,
    backup: NewBackupRecord,
    source_path: Option<PathBuf>,
    artifact: ArtifactManifest,
) -> Result<BackupRecord> {
    let id = Uuid::new_v4().to_string();
    let created_at_ms = Utc::now().timestamp_millis();
    let metadata_json = serde_json::to_string(&backup.backup_metadata)
        .context("Failed to encode backup metadata")?;
    let source_path_text = source_path
        .as_deref()
        .map(|path| path.to_string_lossy().to_string());
    conn.execute(
        "INSERT OR IGNORE INTO backups
         (id, artifact_id, operation_id, provider_id, provider_session_id, session_id,
          source_path, created_at_ms, restore_hint, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            artifact.id,
            backup.operation_id,
            artifact.provider_id,
            artifact.provider_session_id,
            artifact.session_id,
            source_path_text,
            created_at_ms,
            backup.restore_hint,
            metadata_json
        ],
    )
    .context("Failed to insert backup record")?;

    let stored = load_backup_by_artifact_id(conn, &artifact.id)?
        .context("Backup registration did not produce a record")?;
    let expected = BackupRecord {
        id: stored.id.clone(),
        artifact,
        operation_id: backup.operation_id,
        provider_id: stored.provider_id.clone(),
        provider_session_id: stored.provider_session_id.clone(),
        session_id: stored.session_id.clone(),
        source_path,
        created_at_ms: stored.created_at_ms,
        restore_hint: backup.restore_hint,
        metadata: backup.backup_metadata,
    };
    if stored != expected {
        bail!(
            "Backup artifact was already registered with conflicting restore context: {}",
            stored.artifact.path.display()
        );
    }
    Ok(stored)
}

fn load_artifact_by_id(conn: &Connection, artifact_id: &str) -> Result<Option<ArtifactManifest>> {
    conn.query_row(
        "SELECT
            id, artifact_kind, storage_kind, operation_id, provider_id, provider_session_id,
            session_id, projection_report_id, path, content_hash, byte_size,
            mime_type, format, created_at_ms, metadata_json
         FROM artifact_manifests
         WHERE id = ?1",
        [artifact_id],
        decode_artifact_row,
    )
    .optional()
    .context("Failed to load artifact manifest")
}

fn load_artifact_by_registration(
    conn: &Connection,
    artifact_kind: ArtifactManifestKind,
    path: &str,
    content_hash: &str,
    operation_id: Option<&str>,
) -> Result<Option<ArtifactManifest>> {
    conn.query_row(
        "SELECT
            id, artifact_kind, storage_kind, operation_id, provider_id, provider_session_id,
            session_id, projection_report_id, path, content_hash, byte_size,
            mime_type, format, created_at_ms, metadata_json
         FROM artifact_manifests
         WHERE artifact_kind = ?1
           AND path = ?2
           AND content_hash = ?3
           AND operation_id IS ?4",
        params![artifact_kind.as_str(), path, content_hash, operation_id],
        decode_artifact_row,
    )
    .optional()
    .context("Failed to load registered artifact manifest")
}

fn decode_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactManifest> {
    let artifact_kind_text: String = row.get(1)?;
    let storage_kind_text: String = row.get(2)?;
    let metadata_json: String = row.get(14)?;
    let artifact_kind = ArtifactManifestKind::from_str(&artifact_kind_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, error.into())
    })?;
    let storage_kind = ArtifactStorageKind::from_str(&storage_kind_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, error.into())
    })?;
    let metadata = serde_json::from_str(&metadata_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(14, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(ArtifactManifest {
        id: row.get(0)?,
        artifact_kind,
        storage_kind,
        operation_id: row.get(3)?,
        provider_id: row.get(4)?,
        provider_session_id: row.get(5)?,
        session_id: row.get(6)?,
        projection_report_id: row.get(7)?,
        path: PathBuf::from(row.get::<_, String>(8)?),
        content_hash: row.get(9)?,
        byte_size: row.get(10)?,
        mime_type: row.get(11)?,
        format: row.get(12)?,
        created_at_ms: row.get(13)?,
        metadata,
    })
}

fn load_backup_by_id(conn: &Connection, backup_id: &str) -> Result<Option<BackupRecord>> {
    load_backup(conn, "backup.id = ?1", backup_id)
}

fn load_backup_restore(conn: &Connection, restore_id: &str) -> Result<Option<BackupRestoreRecord>> {
    conn.query_row(
        "SELECT id, backup_id, status, actor, started_at_ms, finished_at_ms, error
         FROM backup_restores
         WHERE id = ?1",
        [restore_id],
        decode_backup_restore_row,
    )
    .optional()
    .context("Failed to load backup restore record")
}

fn load_latest_backup_restore(
    conn: &Connection,
    backup_id: &str,
) -> Result<Option<BackupRestoreRecord>> {
    conn.query_row(
        "SELECT id, backup_id, status, actor, started_at_ms, finished_at_ms, error
         FROM backup_restores
         WHERE backup_id = ?1
         ORDER BY started_at_ms DESC, id DESC
         LIMIT 1",
        [backup_id],
        decode_backup_restore_row,
    )
    .optional()
    .context("Failed to load latest backup restore record")
}

fn decode_backup_restore_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BackupRestoreRecord> {
    let status_text: String = row.get(2)?;
    let actor_text: String = row.get(3)?;
    let status = BackupRestoreStatus::from_str(&status_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, error.into())
    })?;
    let actor = ActivityActor::from_str(&actor_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(BackupRestoreRecord {
        id: row.get(0)?,
        backup_id: row.get(1)?,
        status,
        actor,
        started_at_ms: row.get(4)?,
        finished_at_ms: row.get(5)?,
        error: row.get(6)?,
    })
}

fn load_backup_by_artifact_id(
    conn: &Connection,
    artifact_id: &str,
) -> Result<Option<BackupRecord>> {
    load_backup(conn, "backup.artifact_id = ?1", artifact_id)
}

fn load_backup(conn: &Connection, predicate: &str, value: &str) -> Result<Option<BackupRecord>> {
    let sql = format!(
        "SELECT
            backup.id,
            backup.operation_id,
            backup.provider_id,
            backup.provider_session_id,
            backup.session_id,
            backup.source_path,
            backup.created_at_ms,
            backup.restore_hint,
            backup.metadata_json,
            artifact.id,
            artifact.artifact_kind,
            artifact.storage_kind,
            artifact.operation_id,
            artifact.provider_id,
            artifact.provider_session_id,
            artifact.session_id,
            artifact.projection_report_id,
            artifact.path,
            artifact.content_hash,
            artifact.byte_size,
            artifact.mime_type,
            artifact.format,
            artifact.created_at_ms,
            artifact.metadata_json
         FROM backups backup
         JOIN artifact_manifests artifact ON artifact.id = backup.artifact_id
         WHERE {predicate}"
    );
    conn.query_row(&sql, [value], |row| {
        let backup_metadata_json: String = row.get(8)?;
        let artifact_kind_text: String = row.get(10)?;
        let storage_kind_text: String = row.get(11)?;
        let artifact_metadata_json: String = row.get(23)?;
        let artifact_kind =
            ArtifactManifestKind::from_str(&artifact_kind_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    error.into(),
                )
            })?;
        let storage_kind = ArtifactStorageKind::from_str(&storage_kind_text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, error.into())
        })?;
        let backup_metadata = serde_json::from_str(&backup_metadata_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, error.into())
        })?;
        let artifact_metadata = serde_json::from_str(&artifact_metadata_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(23, rusqlite::types::Type::Text, error.into())
        })?;
        Ok(BackupRecord {
            id: row.get(0)?,
            operation_id: row.get(1)?,
            provider_id: row.get(2)?,
            provider_session_id: row.get(3)?,
            session_id: row.get(4)?,
            source_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
            created_at_ms: row.get(6)?,
            restore_hint: row.get(7)?,
            metadata: backup_metadata,
            artifact: ArtifactManifest {
                id: row.get(9)?,
                artifact_kind,
                storage_kind,
                operation_id: row.get(12)?,
                provider_id: row.get(13)?,
                provider_session_id: row.get(14)?,
                session_id: row.get(15)?,
                projection_report_id: row.get(16)?,
                path: PathBuf::from(row.get::<_, String>(17)?),
                content_hash: row.get(18)?,
                byte_size: row.get(19)?,
                mime_type: row.get(20)?,
                format: row.get(21)?,
                created_at_ms: row.get(22)?,
                metadata: artifact_metadata,
            },
        })
    })
    .optional()
    .context("Failed to load backup record")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store::{apply_schema, configure_connection};
    use serde_json::json;

    fn test_connection() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        apply_schema(&mut conn).unwrap();
        conn
    }

    fn new_manifest(path: PathBuf, operation_id: Option<&str>) -> NewArtifactManifest {
        NewArtifactManifest {
            artifact_kind: ArtifactManifestKind::SessionExport,
            operation_id: operation_id.map(str::to_string),
            provider_id: Some("claude".to_string()),
            provider_session_id: Some("provider-session-1".to_string()),
            session_id: None,
            projection_report_id: None,
            path,
            mime_type: Some("application/json".to_string()),
            format: Some("json".to_string()),
            metadata: json!({"source": "test"}),
        }
    }

    struct ArtifactRow<'a> {
        conn: &'a Connection,
        id: &'a str,
        artifact_kind: ArtifactManifestKind,
        path: &'a Path,
        content_hash: &'a str,
        byte_size: i64,
        created_at_ms: i64,
        mime_type: Option<&'a str>,
        format: Option<&'a str>,
        metadata: &'a Value,
    }

    fn insert_artifact_row(row: ArtifactRow<'_>) {
        row.conn
            .execute(
                "INSERT INTO artifact_manifests
             (id, artifact_kind, storage_kind, path, content_hash, byte_size,
              mime_type, format, created_at_ms, metadata_json)
             VALUES (?1, ?2, 'file', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.id,
                    row.artifact_kind.as_str(),
                    row.path.to_string_lossy(),
                    row.content_hash,
                    row.byte_size,
                    row.mime_type,
                    row.format,
                    row.created_at_ms,
                    serde_json::to_string(row.metadata).unwrap(),
                ],
            )
            .unwrap();
    }

    /// Write a managed-layout blob directly to disk for orphan cleanup tests.
    /// Layout: root/ab/cd/<64-char-hash>/name.json
    fn write_managed_blob(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(bytes));
        let blob_dir = root.join(&hash[..2]).join(&hash[2..4]).join(&hash);
        std::fs::create_dir_all(&blob_dir).unwrap();
        let path = blob_dir.join(format!("{name}.json"));
        std::fs::write(&path, bytes).unwrap();
        std::fs::canonicalize(&path).unwrap()
    }

    fn managed_blob_content_hash(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    fn register_test_backup(conn: &mut Connection, dir: &Path, operation_id: &str) -> BackupRecord {
        let source_path = dir.join(format!("{operation_id}-source.jsonl"));
        let backup_path = dir.join(format!("{operation_id}-backup.jsonl"));
        std::fs::write(&source_path, b"session").unwrap();
        std::fs::copy(&source_path, &backup_path).unwrap();
        ArtifactStore::new(conn)
            .register_backup(NewBackupRecord {
                operation_id: Some(operation_id.to_string()),
                provider_id: Some("codex".to_string()),
                provider_session_id: Some(format!("{operation_id}-session")),
                session_id: None,
                source_path: Some(source_path),
                backup_path,
                restore_hint: Some("copy over source".to_string()),
                mime_type: Some("application/x-ndjson".to_string()),
                format: Some("jsonl".to_string()),
                artifact_metadata: json!({"mutation": "rename"}),
                backup_metadata: json!({"mutation": "rename"}),
            })
            .unwrap()
    }

    #[test]
    fn registers_file_with_sha256_and_is_idempotent_per_operation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"hello world").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);

        let first = store
            .register_path(new_manifest(path.clone(), Some("operation-1")))
            .unwrap();
        let second = store
            .register_path(new_manifest(path.clone(), Some("operation-1")))
            .unwrap();
        let third = store
            .register_path(new_manifest(path, Some("operation-2")))
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_ne!(first.id, third.id);
        assert_eq!(first.storage_kind, ArtifactStorageKind::File);
        assert_eq!(first.byte_size, 11);
        assert_eq!(
            first.content_hash,
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert!(first.path.is_absolute());
    }

    #[test]
    fn registers_multiple_paths_atomically_and_resolves_projected_session() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("session.json");
        let markdown_path = dir.path().join("session.md");
        std::fs::write(&json_path, b"{}").unwrap();
        std::fs::write(&markdown_path, b"# Session").unwrap();
        let mut conn = test_connection();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, status, event_count, turn_count,
              projection_version, updated_at_ms)
             VALUES ('session-1', 'claude', 'provider-session-1', 'active', 0, 0, 1, 1)",
            [],
        )
        .unwrap();
        let mut store = ArtifactStore::new(&mut conn);

        let stored = store
            .register_paths(vec![
                new_manifest(json_path, Some("operation-1")),
                NewArtifactManifest {
                    path: markdown_path,
                    mime_type: Some("text/markdown".to_string()),
                    format: Some("md".to_string()),
                    ..new_manifest(PathBuf::new(), Some("operation-1"))
                },
            ])
            .unwrap();

        assert_eq!(stored.len(), 2);
        assert!(stored
            .iter()
            .all(|artifact| artifact.session_id.as_deref() == Some("session-1")));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_manifests WHERE operation_id = 'operation-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn rolls_back_batch_when_any_manifest_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let existing_path = dir.path().join("existing.json");
        let new_path = dir.path().join("new.json");
        std::fs::write(&existing_path, b"existing").unwrap();
        std::fs::write(&new_path, b"new").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);
        store
            .register_path(new_manifest(existing_path.clone(), Some("operation-1")))
            .unwrap();
        let mut conflicting = new_manifest(existing_path, Some("operation-1"));
        conflicting.metadata = json!({"source": "conflict"});

        let error = store
            .register_paths(vec![
                new_manifest(new_path, Some("operation-1")),
                conflicting,
            ])
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("already registered with conflicting context"));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_manifests WHERE operation_id = 'operation-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn registers_directory_with_stable_tree_hash() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backup");
        std::fs::create_dir_all(backup_dir.join("nested")).unwrap();
        std::fs::write(backup_dir.join("a.txt"), b"alpha").unwrap();
        std::fs::write(backup_dir.join("nested/b.txt"), b"beta").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);

        let first = store
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionBackup,
                path: backup_dir.clone(),
                ..new_manifest(backup_dir.clone(), None)
            })
            .unwrap();
        let second = store
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionBackup,
                path: backup_dir.clone(),
                ..new_manifest(backup_dir.clone(), None)
            })
            .unwrap();
        std::fs::write(backup_dir.join("nested/b.txt"), b"changed").unwrap();
        let changed = store
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionBackup,
                path: backup_dir,
                ..new_manifest(PathBuf::new(), None)
            })
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_ne!(first.id, changed.id);
        assert_eq!(first.storage_kind, ArtifactStorageKind::Directory);
        assert_eq!(first.byte_size, 9);
        assert!(first.content_hash.starts_with("sha256-tree-v1:"));
    }

    #[test]
    fn resolves_projection_report_links_to_session_and_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.json");
        std::fs::write(&path, b"{}").unwrap();
        let mut conn = test_connection();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, status, event_count, turn_count, projection_version)
             VALUES ('session-1', 'claude', 'provider-session-1', 'active', 1, 0, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO projection_reports
             (id, session_id, provider_id, operation_kind, projection_version, status,
              summary_json, created_at_ms)
             VALUES ('report-1', 'session-1', 'claude', 'projection', 1, 'complete', '{}', 1)",
            [],
        )
        .unwrap();
        let mut store = ArtifactStore::new(&mut conn);

        let stored = store
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionExport,
                operation_id: None,
                provider_id: None,
                provider_session_id: None,
                session_id: None,
                projection_report_id: Some("report-1".to_string()),
                path,
                mime_type: Some("application/json".to_string()),
                format: Some("json".to_string()),
                metadata: json!({}),
            })
            .unwrap();

        assert_eq!(stored.provider_id.as_deref(), Some("claude"));
        assert_eq!(
            stored.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
        assert_eq!(stored.session_id.as_deref(), Some("session-1"));
        assert_eq!(stored.projection_report_id.as_deref(), Some("report-1"));
    }

    #[test]
    fn rejects_unknown_canonical_session_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"{}").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);
        let mut manifest = new_manifest(path, None);
        manifest.session_id = Some("missing-session".to_string());

        let error = store.register_path(manifest).unwrap_err();

        assert!(error
            .to_string()
            .contains("Artifact session does not exist: missing-session"));
    }

    #[test]
    fn registers_backup_as_artifact_backed_restore_record() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.jsonl");
        let backup_path = dir.path().join("backup.jsonl");
        std::fs::write(&source_path, b"session").unwrap();
        std::fs::copy(&source_path, &backup_path).unwrap();
        let mut conn = test_connection();
        let stored = {
            let mut store = ArtifactStore::new(&mut conn);
            store
                .register_backup(NewBackupRecord {
                    operation_id: Some("operation-1".to_string()),
                    provider_id: Some("codex".to_string()),
                    provider_session_id: Some("provider-session-1".to_string()),
                    session_id: None,
                    source_path: Some(source_path.clone()),
                    backup_path,
                    restore_hint: Some("copy over source".to_string()),
                    mime_type: Some("application/x-ndjson".to_string()),
                    format: Some("jsonl".to_string()),
                    artifact_metadata: json!({"scope": "single_file"}),
                    backup_metadata: json!({"restore": "replace"}),
                })
                .unwrap()
        };

        let reloaded = ArtifactStore::new(&mut conn)
            .get_backup(&stored.id)
            .unwrap()
            .unwrap();
        let legacy_backup_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM pragma_table_info('backups')
                 WHERE name IN ('backup_path', 'content_hash', 'byte_size')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stored, reloaded);
        assert_eq!(
            stored.artifact.artifact_kind,
            ArtifactManifestKind::SessionBackup
        );
        assert_eq!(
            stored.source_path,
            Some(std::fs::canonicalize(source_path).unwrap())
        );
        assert_eq!(
            stored.artifact.mime_type.as_deref(),
            Some("application/x-ndjson")
        );
        assert_eq!(legacy_backup_columns, 0);
    }

    #[test]
    fn registers_backup_without_a_local_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let backup_path = dir.path().join("backup.json");
        std::fs::write(&backup_path, b"{}").unwrap();
        let mut conn = test_connection();
        let stored = ArtifactStore::new(&mut conn)
            .register_backup(NewBackupRecord {
                operation_id: Some("operation-1".to_string()),
                provider_id: Some("database-provider".to_string()),
                provider_session_id: Some("provider-session-1".to_string()),
                session_id: None,
                source_path: None,
                backup_path,
                restore_hint: Some("import canonical JSON".to_string()),
                mime_type: Some("application/json".to_string()),
                format: Some("json".to_string()),
                artifact_metadata: json!({"role": "manager_canonical_backup"}),
                backup_metadata: json!({"restore_mode": "canonical_import"}),
            })
            .unwrap();

        assert_eq!(stored.source_path, None);
        assert_eq!(
            stored.artifact.provider_id.as_deref(),
            Some("database-provider")
        );
        assert_eq!(
            stored.artifact.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
        assert_eq!(stored.artifact.operation_id.as_deref(), Some("operation-1"));
        assert_eq!(stored.operation_id.as_deref(), Some("operation-1"));
    }

    #[test]
    fn persists_backup_restore_lifecycle_and_rejects_second_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = test_connection();
        let backup = register_test_backup(&mut conn, dir.path(), "operation-1");

        let started = ArtifactStore::new(&mut conn)
            .start_backup_restore(&backup.id, ActivityActor::Cli)
            .unwrap();
        assert_eq!(started.status, BackupRestoreStatus::Running);
        assert_eq!(started.finished_at_ms, None);
        assert_eq!(started.error, None);

        let finished = ArtifactStore::new(&mut conn)
            .finish_backup_restore(
                &started.id,
                BackupRestoreStatus::Failed,
                Some("restore failed"),
            )
            .unwrap();
        assert_eq!(finished.status, BackupRestoreStatus::Failed);
        assert!(finished.finished_at_ms.is_some());
        assert_eq!(finished.error.as_deref(), Some("restore failed"));

        let entry = ArtifactStore::new(&mut conn)
            .get_backup_entry(&backup.id)
            .unwrap()
            .unwrap();
        assert_eq!(entry.latest_restore, Some(finished));

        let error = ArtifactStore::new(&mut conn)
            .finish_backup_restore(&started.id, BackupRestoreStatus::Success, None)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Backup restore is missing or already finished"));
    }

    #[test]
    fn rejects_running_as_backup_restore_completion_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = test_connection();
        let backup = register_test_backup(&mut conn, dir.path(), "operation-1");
        let started = ArtifactStore::new(&mut conn)
            .start_backup_restore(&backup.id, ActivityActor::Api)
            .unwrap();

        let error = ArtifactStore::new(&mut conn)
            .finish_backup_restore(&started.id, BackupRestoreStatus::Running, None)
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("completion status must be terminal"));
        assert_eq!(
            ArtifactStore::new(&mut conn)
                .get_backup_entry(&backup.id)
                .unwrap()
                .unwrap()
                .latest_restore
                .unwrap()
                .status,
            BackupRestoreStatus::Running
        );
    }

    #[test]
    fn queries_backups_by_latest_restore_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = test_connection();
        let restored_backup = register_test_backup(&mut conn, dir.path(), "operation-1");
        let untouched_backup = register_test_backup(&mut conn, dir.path(), "operation-2");

        let failed = ArtifactStore::new(&mut conn)
            .start_backup_restore(&restored_backup.id, ActivityActor::Cli)
            .unwrap();
        ArtifactStore::new(&mut conn)
            .finish_backup_restore(
                &failed.id,
                BackupRestoreStatus::Failed,
                Some("first attempt failed"),
            )
            .unwrap();
        conn.execute(
            "UPDATE backup_restores SET started_at_ms = 1 WHERE id = ?1",
            [&failed.id],
        )
        .unwrap();

        let succeeded = ArtifactStore::new(&mut conn)
            .start_backup_restore(&restored_backup.id, ActivityActor::Api)
            .unwrap();
        ArtifactStore::new(&mut conn)
            .finish_backup_restore(&succeeded.id, BackupRestoreStatus::Success, None)
            .unwrap();
        conn.execute(
            "UPDATE backup_restores SET started_at_ms = 2 WHERE id = ?1",
            [&succeeded.id],
        )
        .unwrap();

        let successful = ArtifactStore::new(&mut conn)
            .query_backups(BackupQuery {
                restore_status: Some(BackupRestoreStatus::Success),
                ..Default::default()
            })
            .unwrap();
        let failed = ArtifactStore::new(&mut conn)
            .query_backups(BackupQuery {
                restore_status: Some(BackupRestoreStatus::Failed),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(successful.len(), 1);
        assert_eq!(successful[0].backup.id, restored_backup.id);
        assert_eq!(
            successful[0].latest_restore.as_ref().unwrap().id,
            succeeded.id
        );
        assert!(failed.is_empty());
        assert!(ArtifactStore::new(&mut conn)
            .get_backup_entry(&untouched_backup.id)
            .unwrap()
            .unwrap()
            .latest_restore
            .is_none());
    }

    #[test]
    fn finds_and_deletes_backup_metadata_without_deleting_artifact_path() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source");
        let backup_path = dir.path().join("backup");
        std::fs::create_dir_all(&source_path).unwrap();
        std::fs::create_dir_all(&backup_path).unwrap();
        std::fs::write(backup_path.join("metadata.json"), b"{}").unwrap();
        let mut conn = test_connection();
        let stored = ArtifactStore::new(&mut conn)
            .register_backup(NewBackupRecord {
                operation_id: Some("operation-1".to_string()),
                provider_id: Some("codex".to_string()),
                provider_session_id: None,
                session_id: None,
                source_path: Some(source_path),
                backup_path: backup_path.clone(),
                restore_hint: Some("restore directory".to_string()),
                mime_type: Some("application/vnd.memorph.codex-sync-backup".to_string()),
                format: Some("codex-sync-backup-v1".to_string()),
                artifact_metadata: json!({"role": "codex_prewrite_sync_backup"}),
                backup_metadata: json!({"restore_mode": "codex_sync_restore"}),
            })
            .unwrap();

        let found = ArtifactStore::new(&mut conn)
            .find_backup_by_artifact_path(&backup_path)
            .unwrap()
            .unwrap();
        assert_eq!(found.id, stored.id);
        assert!(ArtifactStore::new(&mut conn)
            .delete_backup_metadata(&stored.id)
            .unwrap());
        assert!(backup_path.exists());
        assert!(ArtifactStore::new(&mut conn)
            .get_backup(&stored.id)
            .unwrap()
            .is_none());
        assert!(ArtifactStore::new(&mut conn)
            .get(&stored.artifact.id)
            .unwrap()
            .is_none());
        assert!(!ArtifactStore::new(&mut conn)
            .delete_backup_metadata(&stored.id)
            .unwrap());
    }

    #[test]
    fn queries_artifacts_by_provider_session_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"{}").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);
        let stored = store
            .register_path(new_manifest(path, Some("operation-1")))
            .unwrap();

        let rows = store
            .query(ArtifactQuery {
                artifact_kind: Some(ArtifactManifestKind::SessionExport),
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("provider-session-1".to_string()),
                ..ArtifactQuery::default()
            })
            .unwrap();

        assert_eq!(rows, vec![stored]);
    }

    #[test]
    fn verifies_artifact_content_and_reports_changes_or_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"original").unwrap();
        let mut conn = test_connection();
        let mut store = ArtifactStore::new(&mut conn);
        let stored = store
            .register_path(new_manifest(path.clone(), Some("operation-1")))
            .unwrap();

        let verified = store.verify(&stored.id).unwrap().unwrap();
        assert_eq!(verified.status, ArtifactVerificationStatus::Verified);
        assert_eq!(
            verified.actual_content_hash.as_deref(),
            Some(stored.content_hash.as_str())
        );
        assert_eq!(verified.actual_byte_size, Some(stored.byte_size));

        std::fs::write(&path, b"changed").unwrap();
        let changed = store.verify(&stored.id).unwrap().unwrap();
        assert_eq!(changed.status, ArtifactVerificationStatus::Changed);
        assert_ne!(
            changed.actual_content_hash.as_deref(),
            Some(stored.content_hash.as_str())
        );

        std::fs::remove_file(path).unwrap();
        let missing = store.verify(&stored.id).unwrap().unwrap();
        assert_eq!(missing.status, ArtifactVerificationStatus::Missing);
        assert_eq!(missing.actual_content_hash, None);
        assert_eq!(missing.actual_byte_size, None);
    }

    #[test]
    fn inspection_reports_verification_and_orphan_layout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sha256");
        let mut conn = test_connection();
        let retained_path = dir.path().join("export.json");
        std::fs::write(&retained_path, b"{}").unwrap();
        let retained = ArtifactStore::new(&mut conn)
            .register_path(new_manifest(retained_path, None))
            .unwrap();
        let orphan_path = write_managed_blob(&root, "orphan", br#"{"orphan":true}"#);
        let loose = root.join("loose.json");
        std::fs::write(&loose, b"not-managed-layout").unwrap();
        let loose_path = std::fs::canonicalize(&loose).unwrap();

        let report = ArtifactStore::new(&mut conn).inspect(&root).unwrap();

        let entry = report
            .registered
            .iter()
            .find(|entry| entry.manifest.id == retained.id)
            .unwrap();
        assert_eq!(entry.retention_state, ArtifactRetentionState::Retained);
        assert_eq!(
            entry.verification.status,
            ArtifactVerificationStatus::Verified
        );
        assert!(!report
            .orphan_files
            .iter()
            .any(|file| file.path == retained.path));
        assert!(report
            .orphan_files
            .iter()
            .any(|file| file.path == orphan_path && file.managed_layout));
        assert!(report
            .orphan_files
            .iter()
            .any(|file| file.path == loose_path && !file.managed_layout));
    }

    #[test]
    fn inspection_is_not_capped_by_interactive_query_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("missing-root");
        let mut conn = test_connection();
        {
            let tx = conn.transaction().unwrap();
            for index in 0..=MAX_QUERY_LIMIT {
                insert_artifact_row(ArtifactRow {
                    conn: &tx,
                    id: &format!("artifact-{index:04}"),
                    artifact_kind: ArtifactManifestKind::SessionExport,
                    path: &dir.path().join(format!("missing-{index:04}.json")),
                    content_hash: &format!("sha256:{index:064x}"),
                    byte_size: 1,
                    created_at_ms: index as i64,
                    mime_type: Some("application/json"),
                    format: Some("json"),
                    metadata: &json!({"source": "inspection-limit-test"}),
                });
            }
            tx.commit().unwrap();
        }

        let report = ArtifactStore::new(&mut conn).inspect(&root).unwrap();

        assert_eq!(report.registered.len(), MAX_QUERY_LIMIT + 1);
        assert!(report
            .registered
            .iter()
            .all(|entry| entry.verification.status == ArtifactVerificationStatus::Missing));
    }

    #[test]
    fn cleanup_dry_run_lists_orphan_files_without_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sha256");
        let mut conn = test_connection();
        let cutoff_ms = Utc::now().timestamp_millis() + 10_000;

        let orphan_path = write_managed_blob(&root, "orphan", b"orphan");
        let loose = root.join("loose.json");
        std::fs::write(&loose, b"loose").unwrap();

        let report = ArtifactStore::new(&mut conn)
            .cleanup_orphan_files(&root, cutoff_ms, false)
            .unwrap();

        assert_eq!(report.candidate_orphan_paths, vec![orphan_path.clone()]);
        assert!(report.deleted_paths.is_empty());
        assert!(report.failures.is_empty());
        assert!(orphan_path.exists());
        assert!(loose.exists());
    }

    #[test]
    fn cleanup_apply_deletes_old_managed_orphan_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sha256");
        let mut conn = test_connection();
        let cutoff_ms = Utc::now().timestamp_millis() + 10_000;

        let orphan_path = write_managed_blob(&root, "orphan", b"orphan");
        let loose = root.join("loose.json");
        std::fs::write(&loose, b"loose").unwrap();

        let report = ArtifactStore::new(&mut conn)
            .cleanup_orphan_files(&root, cutoff_ms, true)
            .unwrap();

        assert_eq!(report.deleted_paths, vec![orphan_path.clone()]);
        assert!(report.failures.is_empty());
        assert!(!orphan_path.exists());
        assert!(loose.exists());
    }

    #[test]
    fn cleanup_skips_files_registered_as_artifact_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sha256");
        let mut conn = test_connection();
        let cutoff_ms = Utc::now().timestamp_millis() + 10_000;

        // A managed-layout blob that is also registered as a manifest.
        let shared_bytes = b"shared";
        let shared_path = write_managed_blob(&root, "shared", shared_bytes);
        let shared_hash = managed_blob_content_hash(shared_bytes);
        insert_artifact_row(ArtifactRow {
            conn: &conn,
            id: "retained",
            artifact_kind: ArtifactManifestKind::SessionExport,
            path: &shared_path,
            content_hash: &shared_hash,
            byte_size: shared_bytes.len() as i64,
            created_at_ms: 1,
            mime_type: Some("application/json"),
            format: Some("json"),
            metadata: &json!({"source": "shared-reference-test"}),
        });
        // A true orphan in the same root.
        let orphan_path = write_managed_blob(&root, "orphan", b"orphan");

        let report = ArtifactStore::new(&mut conn)
            .cleanup_orphan_files(&root, cutoff_ms, true)
            .unwrap();

        // Registered file must not appear in candidates or deleted.
        assert!(!report
            .candidate_orphan_paths
            .iter()
            .any(|path| path == &shared_path));
        assert!(!report.deleted_paths.iter().any(|path| path == &shared_path));
        assert!(shared_path.exists());
        // Orphan is deleted.
        assert!(report.deleted_paths.iter().any(|path| path == &orphan_path));
        assert!(!orphan_path.exists());
    }
}
