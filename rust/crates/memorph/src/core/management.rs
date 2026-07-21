use super::*;

pub fn list_management_activity(query: &ActivityQuery) -> Result<Vec<ActivityRecord>> {
    let conn = local_store::open_database()?;
    ActivityStore::new(&conn).query(query)
}

pub fn inspect_artifacts() -> Result<ArtifactInspectionReport> {
    let mut conn = local_store::open_database()?;
    let root = default_managed_artifact_root()?;
    ArtifactStore::new(&mut conn).inspect(&root)
}

pub fn cleanup_artifacts(
    retention_hours: u64,
    apply: bool,
    actor: ActivityActor,
) -> Result<ArtifactCleanupReport> {
    if retention_hours == 0 {
        anyhow::bail!("Artifact retention must be at least one hour");
    }
    let retention_ms = i64::try_from(retention_hours)
        .ok()
        .and_then(|hours| hours.checked_mul(60 * 60 * 1000))
        .context("Artifact retention exceeds supported range")?;
    let cutoff_ms = chrono::Utc::now()
        .timestamp_millis()
        .checked_sub(retention_ms)
        .context("Artifact retention cutoff is out of range")?;
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::ArtifactCleanup,
        actor,
        summary: if apply {
            "Cleaning orphan artifact files"
        } else {
            "Planning orphan artifact cleanup"
        }
        .to_string(),
        details: serde_json::json!({
            "apply": apply,
            "retention_hours": retention_hours,
            "cutoff_ms": cutoff_ms,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        let root = default_managed_artifact_root()?;
        ArtifactStore::new(&mut conn).cleanup_orphan_files(&root, cutoff_ms, apply)
    })();
    match result {
        Ok(report) => {
            let status = if report.failures.is_empty() {
                ActivityStatus::Success
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: None,
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: if apply {
                        "Cleaned orphan artifact files"
                    } else {
                        "Planned orphan artifact cleanup"
                    }
                    .to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to manage orphan artifact files",
                    serde_json::json!({
                        "apply": apply,
                        "retention_hours": retention_hours,
                        "cutoff_ms": cutoff_ms,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}
