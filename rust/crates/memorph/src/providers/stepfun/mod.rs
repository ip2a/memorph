pub mod adapter;
pub mod hook;

use crate::canonical::ImportedSession;
use crate::provider::{Provider, ProviderCapabilities, ProviderSessionSummary};
use anyhow::{bail, Result};

pub struct StepFunProvider;

impl Provider for StepFunProvider {
    fn id(&self) -> &'static str {
        "stepfun"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: false,
            import: false,
            ..ProviderCapabilities::default()
        }
    }

    fn name(&self) -> &'static str {
        "StepFun"
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        bail!("StepFun native session source is not verified")
    }
    fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
        bail!("StepFun native session source is not verified")
    }
}
