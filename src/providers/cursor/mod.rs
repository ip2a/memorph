mod db;
mod load;
mod scan;
mod write;

use crate::canonical::{CanonicalSession, ExportedSession, ImportedSession};
use crate::provider::{
    canonical_export_result, Provider, ProviderCapabilities, ProviderSessionSummary,
};
use anyhow::Result;
use std::path::Path;

pub struct CursorProvider;

const PROVIDER_ID: &str = "cursor";

impl Provider for CursorProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: false,
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        scan::scan_sessions(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        load::import_session(source_path)
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let session_id = write::export_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        write::delete_session(session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        write::rename_session(session_id, new_title)
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        db::composer_size(session_id)
    }
}
