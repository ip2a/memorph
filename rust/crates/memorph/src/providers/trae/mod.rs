pub mod adapter;
pub mod hook;

use crate::canonical::ImportedSession;
use crate::provider::{Provider, ProviderCapabilities, ProviderSessionSummary};
use anyhow::{bail, Result};

pub struct TraeProvider;

impl Provider for TraeProvider {
    fn id(&self) -> &'static str {
        "trae"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: false,
            import: false,
            ..ProviderCapabilities::default()
        }
    }

    fn name(&self) -> &'static str {
        "TraeCli"
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        bail!("Trae native session source is not verified")
    }
    fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
        bail!("Trae native session source is not verified")
    }
}
