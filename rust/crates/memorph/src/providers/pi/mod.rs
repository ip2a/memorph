pub mod adapter;
pub mod hook;

use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::providers::pi_format;
use crate::session::{Fidelity, ImportedSession};
use anyhow::Result;
use std::path::PathBuf;

pub struct PiProvider;
const PROVIDER_ID: &str = "pi";

impl Provider for PiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "pi"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            storage_shape: StorageShape::Directory,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: false,
                sidecar_files: true,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: false,
                restore: false,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: false,
                runtime_endpoint: false,
                session_activity: false,
            },
            ..ProviderCapabilities::default()
        }
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        pi_format::scan(&roots())
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        pi_format::import(PROVIDER_ID, source_path)
    }
    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        pi_format::fingerprint(PROVIDER_ID, source_path)
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        roots()
    }
}

fn roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|h| h.join(".pi/agent/sessions"))
        .collect()
}
