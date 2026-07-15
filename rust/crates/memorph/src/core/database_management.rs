use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::storage::activity_store::{
    ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStore, NewActivity,
};
use crate::storage::artifact_store::{
    ArtifactManifest, ArtifactManifestKind, ArtifactStore, NewArtifactManifest,
};
use crate::storage::database_backup::{
    self, DatabaseBackupPurpose, VerifiedDatabaseBackup, DATABASE_BACKUP_FORMAT,
    DATABASE_BACKUP_MIME_TYPE,
};
use crate::storage::local_store::{self, LocalSqliteStore};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseBackupReport {
    pub operation_id: String,
    pub backup: VerifiedDatabaseBackup,
    pub artifact: ArtifactManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseRestoreReport {
    pub operation_id: String,
    pub restored_backup: VerifiedDatabaseBackup,
    pub safety_backup: VerifiedDatabaseBackup,
    pub restored_artifact: ArtifactManifest,
    pub safety_artifact: ArtifactManifest,
    pub schema_version: i64,
}

pub fn backup_database(
    output_root: Option<&Path>,
    actor: ActivityActor,
) -> Result<DatabaseBackupReport> {
    backup_database_at(&local_store::database_path()?, output_root, actor)
}

pub fn verify_database_backup(bundle_path: &Path) -> Result<VerifiedDatabaseBackup> {
    database_backup::verify_database_backup_bundle(bundle_path)
}

pub fn restore_database(bundle_path: &Path, actor: ActivityActor) -> Result<DatabaseRestoreReport> {
    restore_database_at(bundle_path, &local_store::database_path()?, None, actor)
}

fn backup_database_at(
    database_path: &Path,
    output_root: Option<&Path>,
    actor: ActivityActor,
) -> Result<DatabaseBackupReport> {
    let mut store = LocalSqliteStore::open(database_path)?;
    let operation_id = ActivityStore::new(store.connection()).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::DatabaseBackup,
        actor,
        summary: "Creating memorph database backup".to_string(),
        details: serde_json::json!({
            "database_path": database_path,
            "output_root": output_root,
        }),
    })?;

    let result = (|| {
        let backup = database_backup::create_database_backup_bundle(
            store.connection(),
            database_path,
            output_root,
            DatabaseBackupPurpose::Manual,
            actor,
        )?;
        let artifact =
            register_database_backup_artifact(store.connection_mut(), &operation_id, &backup)?;
        Ok(DatabaseBackupReport {
            operation_id: operation_id.clone(),
            backup,
            artifact,
        })
    })();

    match result {
        Ok(report) => {
            ActivityStore::new(store.connection()).finish(
                &operation_id,
                ActivityCompletion::success(
                    "Created memorph database backup",
                    serde_json::json!({
                        "bundle_path": report.backup.bundle_path,
                        "artifact_id": report.artifact.id,
                        "schema_version": report.backup.manifest.schema_version,
                        "database_bytes": report.backup.manifest.database_bytes,
                    }),
                ),
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(store.connection()).finish(
                &operation_id,
                ActivityCompletion::failed(
                    "Failed to create memorph database backup",
                    serde_json::json!({
                        "database_path": database_path,
                        "output_root": output_root,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

fn restore_database_at(
    bundle_path: &Path,
    database_path: &Path,
    safety_output_root: Option<&Path>,
    actor: ActivityActor,
) -> Result<DatabaseRestoreReport> {
    let restored_backup = verify_database_backup(bundle_path)?;
    let mut store = LocalSqliteStore::open(database_path)?;
    ensure_no_live_server(store.connection())?;

    let original_operation_id = ActivityStore::new(store.connection()).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::DatabaseRestore,
        actor,
        summary: "Restoring memorph database".to_string(),
        details: serde_json::json!({
            "bundle_path": restored_backup.bundle_path,
            "backup_id": restored_backup.manifest.backup_id,
            "database_path": database_path,
        }),
    })?;

    let safety_backup = match database_backup::create_database_backup_bundle(
        store.connection(),
        database_path,
        safety_output_root,
        DatabaseBackupPurpose::PreRestore,
        actor,
    ) {
        Ok(backup) => backup,
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(store.connection()).finish(
                &original_operation_id,
                ActivityCompletion::failed(
                    "Failed to create pre-restore safety backup",
                    serde_json::json!({
                        "bundle_path": restored_backup.bundle_path,
                        "database_path": database_path,
                    }),
                    &message,
                ),
            )?;
            return Err(error).context("Database restore stopped before replacing local state");
        }
    };

    if let Err(error) =
        database_backup::restore_database_from_backup(store.connection_mut(), &restored_backup)
    {
        return rollback_failed_restore(
            &mut store,
            &original_operation_id,
            &restored_backup,
            &safety_backup,
            error.context("Failed to restore selected database backup"),
        );
    }

    match finalize_successful_restore(&mut store, &restored_backup, &safety_backup, actor) {
        Ok(report) => Ok(report),
        Err(error) => rollback_failed_restore(
            &mut store,
            &original_operation_id,
            &restored_backup,
            &safety_backup,
            error.context("Restored database could not be finalized"),
        ),
    }
}

fn finalize_successful_restore(
    store: &mut LocalSqliteStore,
    restored_backup: &VerifiedDatabaseBackup,
    safety_backup: &VerifiedDatabaseBackup,
    actor: ActivityActor,
) -> Result<DatabaseRestoreReport> {
    let operation_id = ActivityStore::new(store.connection()).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::DatabaseRestore,
        actor,
        summary: "Finalizing restored memorph database".to_string(),
        details: serde_json::json!({
            "bundle_path": restored_backup.bundle_path,
            "backup_id": restored_backup.manifest.backup_id,
            "safety_bundle_path": safety_backup.bundle_path,
        }),
    })?;
    let restored_artifact =
        register_database_backup_artifact(store.connection_mut(), &operation_id, restored_backup)?;
    let safety_artifact =
        register_database_backup_artifact(store.connection_mut(), &operation_id, safety_backup)?;
    let schema_version = local_store::current_schema_version();
    ActivityStore::new(store.connection()).finish(
        &operation_id,
        ActivityCompletion::success(
            "Restored memorph database",
            serde_json::json!({
                "bundle_path": restored_backup.bundle_path,
                "backup_id": restored_backup.manifest.backup_id,
                "safety_bundle_path": safety_backup.bundle_path,
                "restored_artifact_id": restored_artifact.id,
                "safety_artifact_id": safety_artifact.id,
                "schema_version": schema_version,
            }),
        ),
    )?;
    Ok(DatabaseRestoreReport {
        operation_id,
        restored_backup: restored_backup.clone(),
        safety_backup: safety_backup.clone(),
        restored_artifact,
        safety_artifact,
        schema_version,
    })
}

fn rollback_failed_restore(
    store: &mut LocalSqliteStore,
    original_operation_id: &str,
    restored_backup: &VerifiedDatabaseBackup,
    safety_backup: &VerifiedDatabaseBackup,
    restore_error: anyhow::Error,
) -> Result<DatabaseRestoreReport> {
    let restore_message = format!("{restore_error:#}");
    if let Err(rollback_error) =
        database_backup::restore_database_from_backup(store.connection_mut(), safety_backup)
    {
        bail!(
            "Database restore failed: {restore_message}. Safety rollback also failed: {rollback_error:#}"
        );
    }

    let registration_result = (|| {
        register_database_backup_artifact(
            store.connection_mut(),
            original_operation_id,
            restored_backup,
        )?;
        register_database_backup_artifact(
            store.connection_mut(),
            original_operation_id,
            safety_backup,
        )?;
        Ok::<_, anyhow::Error>(())
    })();
    let completion_result = ActivityStore::new(store.connection()).finish(
        original_operation_id,
        ActivityCompletion::failed(
            "Database restore failed; restored pre-restore safety backup",
            serde_json::json!({
                "bundle_path": restored_backup.bundle_path,
                "backup_id": restored_backup.manifest.backup_id,
                "safety_bundle_path": safety_backup.bundle_path,
                "rollback_completed": true,
            }),
            &restore_message,
        ),
    );

    if let Err(error) = registration_result {
        bail!(
            "Database restore failed and safety rollback completed, but backup artifacts could not be registered: {error:#}. Original error: {restore_message}"
        );
    }
    if let Err(error) = completion_result {
        bail!(
            "Database restore failed and safety rollback completed, but failure activity could not be recorded: {error:#}. Original error: {restore_message}"
        );
    }
    Err(restore_error)
}

fn register_database_backup_artifact(
    conn: &mut Connection,
    operation_id: &str,
    backup: &VerifiedDatabaseBackup,
) -> Result<ArtifactManifest> {
    ArtifactStore::new(conn).register_path(NewArtifactManifest {
        artifact_kind: ArtifactManifestKind::DatabaseBackup,
        operation_id: Some(operation_id.to_string()),
        provider_id: None,
        provider_session_id: None,
        session_id: None,
        projection_report_id: None,
        path: backup.bundle_path.clone(),
        mime_type: Some(DATABASE_BACKUP_MIME_TYPE.to_string()),
        format: Some(DATABASE_BACKUP_FORMAT.to_string()),
        metadata: serde_json::to_value(&backup.manifest)
            .context("Failed to encode database backup artifact metadata")?,
    })
}

fn ensure_no_live_server(conn: &Connection) -> Result<()> {
    let pid = conn
        .query_row(
            "SELECT pid
             FROM runtime_endpoints
             WHERE runtime_kind = 'hook_server' AND pid IS NOT NULL
             ORDER BY last_seen_at_ms DESC
             LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .context("Failed to inspect active memorph server state")?;
    let Some(pid) = pid else {
        return Ok(());
    };
    let pid = u32::try_from(pid).context("Stored memorph server PID is invalid")?;
    if crate::hooks::lifecycle::pid_is_alive(pid) {
        bail!("Refusing to restore memorph.db while memorph server process {pid} is running");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::activity_store::{ActivityQuery, ActivityStatus};
    use crate::storage::artifact_store::ArtifactQuery;

    fn insert_group(conn: &Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO sync_groups
             (id, title, source_provider, created_at_ms, updated_at_ms, status)
             VALUES (?1, ?2, 'claude', 1, 1, 'active')",
            [id, title],
        )
        .unwrap();
    }

    fn group_title(conn: &Connection, id: &str) -> Option<String> {
        conn.query_row("SELECT title FROM sync_groups WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
    }

    #[test]
    fn backup_registers_artifact_and_terminal_activity() {
        let dir = tempfile::tempdir().unwrap();
        let database_path = dir.path().join("memorph.db");
        let store = LocalSqliteStore::open(&database_path).unwrap();
        insert_group(store.connection(), "current", "Current");
        drop(store);

        let report = backup_database_at(
            &database_path,
            Some(&dir.path().join("backups")),
            ActivityActor::Cli,
        )
        .unwrap();
        let mut store = LocalSqliteStore::open(&database_path).unwrap();
        let artifacts = ArtifactStore::new(store.connection_mut())
            .query(ArtifactQuery {
                artifact_kind: Some(ArtifactManifestKind::DatabaseBackup),
                operation_id: Some(report.operation_id.clone()),
                ..ArtifactQuery::default()
            })
            .unwrap();
        let activities = ActivityStore::new(store.connection())
            .query(&ActivityQuery {
                operation_kind: Some(ActivityOperationKind::DatabaseBackup),
                status: Some(ActivityStatus::Success),
                ..ActivityQuery::default()
            })
            .unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].id, report.operation_id);
    }

    #[test]
    fn restore_replaces_state_and_registers_safety_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let source = LocalSqliteStore::open(&source_path).unwrap();
        insert_group(source.connection(), "restored", "Restored");
        let selected = database_backup::create_database_backup_bundle(
            source.connection(),
            &source_path,
            Some(&dir.path().join("selected")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();

        let destination_path = dir.path().join("destination.db");
        let destination = LocalSqliteStore::open(&destination_path).unwrap();
        insert_group(destination.connection(), "current", "Current");
        drop(destination);

        let report = restore_database_at(
            &selected.bundle_path,
            &destination_path,
            Some(&dir.path().join("safety")),
            ActivityActor::Cli,
        )
        .unwrap();
        let mut restored = LocalSqliteStore::open(&destination_path).unwrap();
        let artifacts = ArtifactStore::new(restored.connection_mut())
            .query(ArtifactQuery {
                artifact_kind: Some(ArtifactManifestKind::DatabaseBackup),
                operation_id: Some(report.operation_id.clone()),
                ..ArtifactQuery::default()
            })
            .unwrap();
        let activities = ActivityStore::new(restored.connection())
            .query(&ActivityQuery {
                operation_kind: Some(ActivityOperationKind::DatabaseRestore),
                status: Some(ActivityStatus::Success),
                ..ActivityQuery::default()
            })
            .unwrap();

        assert_eq!(
            group_title(restored.connection(), "restored").as_deref(),
            Some("Restored")
        );
        assert_eq!(group_title(restored.connection(), "current"), None);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(activities.len(), 1);
        assert_eq!(
            report.safety_backup.manifest.purpose,
            DatabaseBackupPurpose::PreRestore
        );
    }

    #[test]
    fn restore_rejects_live_memorph_server() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let source = LocalSqliteStore::open(&source_path).unwrap();
        let selected = database_backup::create_database_backup_bundle(
            source.connection(),
            &source_path,
            Some(&dir.path().join("selected")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();
        let destination_path = dir.path().join("destination.db");
        let destination = LocalSqliteStore::open(&destination_path).unwrap();
        destination
            .connection()
            .execute(
                "INSERT INTO runtime_endpoints
                 (id, runtime_kind, pid, published_at_ms, last_seen_at_ms, metadata_json)
                 VALUES ('server', 'hook_server', ?1, 1, 1, '{}')",
                [i64::from(std::process::id())],
            )
            .unwrap();
        drop(destination);

        let error = restore_database_at(
            &selected.bundle_path,
            &destination_path,
            Some(&dir.path().join("safety")),
            ActivityActor::Cli,
        )
        .unwrap_err();

        assert!(error.to_string().contains("server process"));
        assert!(!dir.path().join("safety").exists());
    }

    #[test]
    fn failed_finalization_rolls_back_to_safety_backup() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.db");
        let source = LocalSqliteStore::open(&source_path).unwrap();
        insert_group(source.connection(), "restored", "Restored");
        source
            .connection()
            .execute_batch(
                "CREATE TRIGGER reject_database_artifact
                 BEFORE INSERT ON artifact_manifests
                 WHEN NEW.artifact_kind = 'database_backup'
                 BEGIN
                     SELECT RAISE(ABORT, 'forced artifact failure');
                 END;",
            )
            .unwrap();
        let selected = database_backup::create_database_backup_bundle(
            source.connection(),
            &source_path,
            Some(&dir.path().join("selected")),
            DatabaseBackupPurpose::Manual,
            ActivityActor::Cli,
        )
        .unwrap();

        let destination_path = dir.path().join("destination.db");
        let destination = LocalSqliteStore::open(&destination_path).unwrap();
        insert_group(destination.connection(), "current", "Current");
        drop(destination);

        let error = restore_database_at(
            &selected.bundle_path,
            &destination_path,
            Some(&dir.path().join("safety")),
            ActivityActor::Cli,
        )
        .unwrap_err();
        let restored = LocalSqliteStore::open(&destination_path).unwrap();
        let failed = ActivityStore::new(restored.connection())
            .query(&ActivityQuery {
                operation_kind: Some(ActivityOperationKind::DatabaseRestore),
                status: Some(ActivityStatus::Failed),
                ..ActivityQuery::default()
            })
            .unwrap();

        assert!(format!("{error:#}").contains("forced artifact failure"));
        assert_eq!(
            group_title(restored.connection(), "current").as_deref(),
            Some("Current")
        );
        assert_eq!(group_title(restored.connection(), "restored"), None);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].details["rollback_completed"], true);
    }
}
