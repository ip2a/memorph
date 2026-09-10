//! Amazon Q CLI provider backed by `data_local_dir()/amazon-q/data.sqlite3`.

use crate::provider::{
    Provider, ProviderCapabilities, ProviderContentFidelity, ProviderSessionImportPage,
    ProviderSessionSummary, ProviderSourceFingerprint, ScanStrategy, StorageShape, TurnQuality,
};
use crate::session::{
    Context, Identity, ImportedSession, MappingDirection, MappingReport, Provenance, ProviderRef,
    Schema, Session,
};
use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct AmazonQProvider;
const PROVIDER_ID: &str = "amazonq";
const SOURCE_PREFIX: &str = "amazonq://";

impl Provider for AmazonQProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Amazon Q"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            scan_strategy: ScanStrategy::Indexed,
            page_strategy: crate::provider::PageStrategy::FullImport,
            storage_shape: StorageShape::Sqlite,
            turn_quality: TurnQuality::Exact,
            import_fidelity: ProviderContentFidelity {
                text: Some(crate::session::Fidelity::Preserved),
                tool_call: Some(crate::session::Fidelity::Preserved),
                tool_result: Some(crate::session::Fidelity::Preserved),
                provider_payload: Some(crate::session::Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let Some(path) = db_path() else {
            return Ok(Vec::new());
        };
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let conn = open_db(&path)?;
        let mut stmt = conn.prepare("SELECT key, value FROM conversations")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows.flatten() {
            let (key, value) = row;
            let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
            let events = crate::providers::q_conversation::parse_history(
                PROVIDER_ID,
                &value,
                &key,
                &mut report,
            );
            if events.is_empty() {
                continue;
            }
            let title = crate::providers::q_conversation::first_prompt_text_value(
                &serde_json::from_str(&value).unwrap_or_default(),
                100,
            );
            let (_, last) = crate::providers::q_conversation::history_time_bounds(&value);
            out.push(ProviderSessionSummary {
                archived: false,
                session_id: key.clone(),
                title,
                project_dir: Some(key.clone()),
                created_at: last.map(|v| v.timestamp_millis()),
                last_active_at: last.map(|v| v.timestamp_millis()),
                source_path: Some(format!("{SOURCE_PREFIX}{key}")),
            });
        }
        Ok(out)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let key = source_path
            .strip_prefix(SOURCE_PREFIX)
            .unwrap_or(source_path);
        let path = db_path().context("Amazon Q database path unavailable")?;
        let conn = open_db(&path)?;
        let value: String = conn
            .query_row(
                "SELECT value FROM conversations WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?
            .context("Amazon Q conversation not found")?;
        let json: serde_json::Value = serde_json::from_str(&value)?;
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events = crate::providers::q_conversation::parse_history_value(
            PROVIDER_ID,
            &json,
            key,
            &mut report,
        );
        let (created_at, last_active_at) =
            crate::providers::q_conversation::history_time_bounds_value(&json);
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect();
        Ok(ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity {
                    id: key.to_string(),
                    title: crate::providers::q_conversation::first_prompt_text_value(&json, 100),
                },
                context: Context {
                    workspace: Some(key.to_string()),
                    created_at,
                    last_active_at,
                    tags: Vec::new(),
                },
                events,
                extensions: BTreeMap::new(),
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".into()),
                primary_source: ProviderRef {
                    provider_id: PROVIDER_ID.into(),
                    session_id: key.into(),
                    source_path: Some(format!("{SOURCE_PREFIX}{key}")),
                },
                aliases: Vec::new(),
            },
            event_meta,
            report,
        })
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        let imported = self.import_session(source_path)?;
        let all = imported.session.events.clone();
        let event_count = all.len();
        let message_count = all
            .iter()
            .filter(|e| crate::provider::event_is_visible_message(e))
            .count();
        let turns = crate::session_projection::project_session_turns(
            &imported.session.identity.id,
            &all,
            TurnQuality::Exact,
        );
        let page_events: Vec<_> = all
            .into_iter()
            .skip(event_offset)
            .take(event_limit.unwrap_or(usize::MAX))
            .collect();
        let page_turns = crate::session_projection::project_session_turns(
            &imported.session.identity.id,
            &page_events,
            TurnQuality::Inferred,
        );
        Ok(ProviderSessionImportPage {
            imported: ImportedSession {
                session: Session {
                    events: page_events,
                    ..imported.session
                },
                ..imported
            },
            event_count,
            message_count,
            turn_count: Some(turns.len()),
            turns: page_turns,
        })
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        let key = source_path
            .strip_prefix(SOURCE_PREFIX)
            .unwrap_or(source_path);
        let Some(path) = db_path() else {
            return Ok(None);
        };
        let metadata = std::fs::metadata(&path)?;
        let mut hash = Sha256::new();
        hash.update(key.as_bytes());
        hash.update(std::fs::read(&path)?);
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .min(i64::MAX as u128) as i64,
            size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            value: format!("amazonq-v1:{key}:{:x}", hash.finalize()),
        }))
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        db_path().into_iter().collect()
    }
}

fn db_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("amazon-q")
            .join("data.sqlite3"),
    )
}
fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}
