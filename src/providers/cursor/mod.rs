mod db;
mod load;
mod scan;
mod write;

use crate::model::{MemorphSession, SessionMeta};
use crate::provider::{Provider, ProviderCapabilities};
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
            load: true,
            write: true,
            delete: true,
            rename: true,
            resume: false,
        }
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        scan::scan_sessions(None)
    }

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        load::load_session(source_path)
    }

    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        write::write_session(session, target_dir)
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
