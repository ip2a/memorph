//! Oh My Pi (`omp`): same pi-ai JSONL family as Pi, sessions under
//! `~/.omp/agent/sessions/<encoded-cwd>/<ts>_<uuid>.jsonl`. Files may carry a
//! leading `{"type":"title"}` row; the shared parser locates the session
//! header by type. Parsing lives in [`crate::providers::pi_format`].

use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::providers::pi_format;
use crate::session::{Fidelity, ImportedSession};
use anyhow::Result;
use std::path::PathBuf;

pub struct OmpProvider;
const PROVIDER_ID: &str = "omp";

impl Provider for OmpProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Oh My Pi"
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
        .map(|h| h.join(".omp/agent/sessions"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_omp_transcript_with_leading_title_row() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("2026-09-24T08-20-25-247Z_01a0d280.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"title","v":1,"title":"","updatedAt":"2026-09-24T08:20:25.247Z"}"#,
                "\n",
                r#"{"type":"session","version":3,"id":"01a0d280","timestamp":"2026-09-24T08:20:25.247Z","cwd":"/tmp/omp-project"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"user","content":"hello"},"timestamp":"2026-09-24T08:20:26.000Z"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"hi there"}]},"timestamp":"2026-09-24T08:20:27.000Z"}"#,
                "\n",
            ),
        )
        .unwrap();

        let imported = pi_format::import(PROVIDER_ID, file.to_str().unwrap()).unwrap();
        assert_eq!(imported.session.identity.id, "01a0d280");
        assert_eq!(imported.session.context.workspace.as_deref(), Some("/tmp/omp-project"));
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(
            imported.provenance.primary_source.provider_id, PROVIDER_ID
        );
    }
}
