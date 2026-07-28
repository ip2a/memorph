use crate::sync::{Holding, SyncGroup, SyncReport};
use anyhow::{Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

pub fn list_groups(conn: &Connection) -> Result<Vec<SyncGroup>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, source_provider, created_at_ms, updated_at_ms
             FROM sync_groups
             WHERE status = 'active'
             ORDER BY updated_at_ms DESC, id",
        )
        .context("Failed to prepare sync group list query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .context("Failed to query sync groups")?;

    let mut groups = Vec::new();
    for row in rows {
        let (id, title, source_provider, created_at, updated_at) =
            row.context("Failed to decode sync group row")?;
        let holdings = load_holdings(conn, &id, source_provider.as_deref())?;
        groups.push(SyncGroup {
            id,
            title,
            source_provider,
            created_at,
            updated_at,
            holdings,
        });
    }
    Ok(groups)
}

pub fn load_group(conn: &Connection, id: &str) -> Result<SyncGroup> {
    let group = conn
        .query_row(
            "SELECT id, title, source_provider, created_at_ms, updated_at_ms
             FROM sync_groups
             WHERE id = ?1
               AND status = 'active'",
            [id],
            |row| {
                Ok(SyncGroup {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source_provider: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    holdings: Vec::new(),
                })
            },
        )
        .optional()
        .with_context(|| format!("Failed to load sync group: {id}"))?
        .with_context(|| format!("Sync group not found: {id}"))?;
    let holdings = load_holdings(conn, &group.id, group.source_provider.as_deref())?;
    Ok(SyncGroup { holdings, ..group })
}

pub fn save_group(conn: &mut Connection, group: &SyncGroup) -> Result<()> {
    let tx = conn
        .transaction()
        .context("Failed to start sync group transaction")?;
    tx.execute(
        "INSERT INTO sync_groups
         (id, title, source_provider, created_at_ms, updated_at_ms, status)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active')
         ON CONFLICT(id) DO UPDATE SET
          title = excluded.title,
          source_provider = excluded.source_provider,
          created_at_ms = excluded.created_at_ms,
          updated_at_ms = excluded.updated_at_ms,
          status = 'active'",
        params![
            group.id,
            group.title,
            group.source_provider,
            group.created_at,
            group.updated_at
        ],
    )
    .context("Failed to upsert sync group")?;
    for holding in &group.holdings {
        tx.execute(
            "INSERT INTO sync_holdings
             (id, group_id, provider_id, session_id, provider_session_id, target_dir,
              created_at_ms, last_active_at_ms, last_sync_at_ms, last_sync_from, last_error)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
              group_id = excluded.group_id,
              provider_id = excluded.provider_id,
              provider_session_id = excluded.provider_session_id,
              target_dir = excluded.target_dir,
              created_at_ms = excluded.created_at_ms,
              last_active_at_ms = excluded.last_active_at_ms,
              last_sync_at_ms = excluded.last_sync_at_ms,
              last_sync_from = excluded.last_sync_from,
              last_error = excluded.last_error",
            params![
                holding.id,
                group.id,
                holding.provider,
                holding.session_id,
                holding.target_dir,
                holding.created_at,
                holding.last_active_at,
                holding.last_sync_at,
                holding.last_sync_from,
                holding.last_error
            ],
        )
        .context("Failed to upsert sync holding")?;
    }

    let current_holding_ids = group
        .holdings
        .iter()
        .map(|holding| holding.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let existing_holding_ids = {
        let mut stmt = tx
            .prepare("SELECT id FROM sync_holdings WHERE group_id = ?1")
            .context("Failed to prepare existing sync holding query")?;
        let rows = stmt
            .query_map([group.id.as_str()], |row| row.get::<_, String>(0))
            .context("Failed to query existing sync holdings")?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.context("Failed to decode existing sync holding id")?);
        }
        ids
    };
    for existing_id in existing_holding_ids {
        if current_holding_ids.contains(existing_id.as_str()) {
            continue;
        }
        tx.execute(
            "DELETE FROM sync_holdings WHERE group_id = ?1 AND id = ?2",
            params![group.id.as_str(), existing_id.as_str()],
        )
        .context("Failed to delete removed sync holding")?;
    }
    tx.commit()
        .context("Failed to commit sync group transaction")
}

pub fn delete_group(conn: &Connection, group_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_groups SET status = 'deleted' WHERE id = ?1",
        [group_id],
    )
    .with_context(|| format!("Failed to delete sync group: {group_id}"))?;
    Ok(())
}

pub fn record_sync_run(
    conn: &Connection,
    group_id: &str,
    source_holding_id: &str,
    started_at_ms: i64,
    finished_at_ms: i64,
    report: &SyncReport,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let status = if report.errors.is_empty() {
        "success"
    } else if report.success.is_empty() {
        "failed"
    } else {
        "partial"
    };
    let result_json = serde_json::to_string(&json!({
        "source_provider": report.source_provider,
        "source_holding_id": report.source_holding_id,
        "success": report.success,
        "errors": report.errors,
        "target_assessments": report.target_assessments,
    }))
    .context("Failed to encode sync run result")?;
    let error = (!report.errors.is_empty()).then(|| report.errors.join("\n"));
    conn.execute(
        "INSERT INTO sync_runs
         (id, group_id, source_holding_id, status, started_at_ms, finished_at_ms, result_json, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            group_id,
            source_holding_id,
            status,
            started_at_ms,
            finished_at_ms,
            result_json,
            error
        ],
    )
    .context("Failed to insert sync run")?;
    Ok(id)
}

fn load_holdings(
    conn: &Connection,
    group_id: &str,
    source_provider: Option<&str>,
) -> Result<Vec<Holding>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, provider_id, provider_session_id, target_dir,
                    created_at_ms, last_active_at_ms, last_sync_at_ms, last_sync_from, last_error
             FROM sync_holdings
             WHERE group_id = ?1
             ORDER BY
               CASE WHEN ?2 IS NOT NULL AND provider_id = ?2 THEN 0 ELSE 1 END,
               created_at_ms,
               id",
        )
        .context("Failed to prepare sync holding query")?;
    let rows = stmt
        .query_map(params![group_id, source_provider], |row| {
            Ok(Holding {
                id: row.get(0)?,
                provider: row.get(1)?,
                session_id: row.get(2)?,
                target_dir: row.get(3)?,
                created_at: row.get(4)?,
                last_active_at: row.get(5)?,
                last_sync_at: row.get(6)?,
                last_sync_from: row.get(7)?,
                last_error: row.get(8)?,
            })
        })
        .context("Failed to query sync holdings")?;
    let mut holdings = Vec::new();
    for row in rows {
        holdings.push(row.context("Failed to decode sync holding row")?);
    }
    Ok(holdings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store;

    #[test]
    fn saves_lists_loads_and_deletes_sync_groups() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let group = sample_group();

        save_group(&mut conn, &group).unwrap();

        let groups = list_groups(&conn).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "group-1");
        assert_eq!(groups[0].holdings.len(), 2);
        assert_eq!(groups[0].holdings[0].provider, "claude");

        let loaded = load_group(&conn, "group-1").unwrap();
        assert_eq!(loaded.title, "Project sync");
        assert_eq!(loaded.holdings[1].session_id, "codex-session");

        delete_group(&conn, "group-1").unwrap();
        assert!(list_groups(&conn).unwrap().is_empty());
        assert!(load_group(&conn, "group-1").is_err());
    }

    #[test]
    fn records_sync_runs_with_report_status() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        save_group(&mut conn, &sample_group()).unwrap();
        let report = SyncReport {
            source_provider: "claude".to_string(),
            source_holding_id: "holding-source".to_string(),
            success: vec!["codex".to_string()],
            errors: Vec::new(),
            target_assessments: vec![crate::sync::SyncTargetAssessment {
                provider: "codex".to_string(),
                fidelity: crate::session::Fidelity::Normalized,
                write_risk: crate::providers::find_provider("codex")
                    .unwrap()
                    .capabilities()
                    .write_risk,
            }],
        };

        let run_id = record_sync_run(&conn, "group-1", "holding-source", 10, 20, &report).unwrap();

        let row: (String, String, String) = conn
            .query_row(
                "SELECT status, source_holding_id, result_json FROM sync_runs WHERE id = ?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "success");
        assert_eq!(row.1, "holding-source");
        assert!(row.2.contains("\"codex\""));
        assert!(row.2.contains("\"target_assessments\""));
        assert!(row.2.contains("\"fidelity\":\"normalized\""));
        assert!(row.2.contains("\"level\":\"high\""));
    }

    #[test]
    fn preserving_existing_holdings_keeps_sync_run_source_links() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut group = sample_group();
        save_group(&mut conn, &group).unwrap();
        let report = SyncReport {
            source_provider: "claude".to_string(),
            source_holding_id: "holding-source".to_string(),
            success: vec!["codex".to_string()],
            errors: Vec::new(),
            target_assessments: Vec::new(),
        };
        let run_id = record_sync_run(&conn, "group-1", "holding-source", 10, 20, &report).unwrap();

        group.title = "Renamed".to_string();
        group.updated_at = 30;
        save_group(&mut conn, &group).unwrap();

        let source_holding_id: Option<String> = conn
            .query_row(
                "SELECT source_holding_id FROM sync_runs WHERE id = ?1",
                [run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_holding_id.as_deref(), Some("holding-source"));
    }

    fn sample_group() -> SyncGroup {
        SyncGroup {
            id: "group-1".to_string(),
            title: "Project sync".to_string(),
            source_provider: Some("claude".to_string()),
            created_at: 1,
            updated_at: 2,
            holdings: vec![
                Holding {
                    id: "holding-source".to_string(),
                    provider: "claude".to_string(),
                    session_id: "claude-session".to_string(),
                    target_dir: Some("/tmp/project".to_string()),
                    created_at: 1,
                    last_active_at: Some(2),
                    last_sync_at: Some(3),
                    last_sync_from: Some("claude".to_string()),
                    last_error: None,
                },
                Holding {
                    id: "holding-target".to_string(),
                    provider: "codex".to_string(),
                    session_id: "codex-session".to_string(),
                    target_dir: Some("/tmp/project".to_string()),
                    created_at: 1,
                    last_active_at: None,
                    last_sync_at: None,
                    last_sync_from: None,
                    last_error: Some("old error".to_string()),
                },
            ],
        }
    }
}
