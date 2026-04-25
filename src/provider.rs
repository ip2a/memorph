use crate::model::{MemorphSession, SessionMeta};
use anyhow::Result;
use std::path::Path;

/// Provider trait: each AI coding tool implements this interface
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;

    /// Scan all session metadata
    fn scan_sessions(&self) -> Result<Vec<SessionMeta>>;

    /// Load full messages for a given session
    fn load_session(&self, source_path: &str) -> Result<MemorphSession>;

    /// Write session to target tool directory (experimental)
    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        let _ = session;
        let _ = target_dir;
        anyhow::bail!("Write not supported for provider: {}", self.id())
    }

    /// Delete a session
    fn delete_session(&self, session_id: &str) -> Result<()> {
        let _ = session_id;
        anyhow::bail!("Delete not supported for provider: {}", self.id())
    }

    /// Rename a session
    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let _ = session_id;
        let _ = new_title;
        anyhow::bail!("Rename not supported for provider: {}", self.id())
    }
}
