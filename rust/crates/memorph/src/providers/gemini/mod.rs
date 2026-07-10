pub mod adapter;
pub mod hook;

use crate::canonical::ImportedSession;
use crate::provider::{Provider, ProviderCapabilities, ProviderSessionSummary};
use crate::providers::generic_json::{self, JsonProviderSpec};
use anyhow::Result;
use std::path::PathBuf;

pub struct GeminiProvider;

const PROVIDER_ID: &str = "gemini";

impl Provider for GeminiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: false,
            delete: false,
            rename: false,
            resume: true,
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        generic_json::scan_sessions(spec())
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        generic_json::import_session(spec(), source_path)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("gemini --resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let path = PathBuf::from(session_id);
        if path.exists() {
            return Ok(std::fs::metadata(path)?.len());
        }
        Ok(0)
    }
}

fn spec() -> JsonProviderSpec {
    JsonProviderSpec {
        provider_id: PROVIDER_ID,
        extension_key: "gemini_session",
        roots: gemini_roots,
    }
}

fn gemini_roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| vec![home.join(".gemini").join("tmp")])
        .unwrap_or_default()
}
