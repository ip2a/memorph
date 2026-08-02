use crate::provider::{Provider, ProviderCapabilities, ProviderSessionSummary};
use crate::providers::generic_json::{self, JsonProviderSpec};
use crate::session::ImportedSession;
use anyhow::Result;
use std::path::PathBuf;

fn spec(provider_id: &'static str, roots: fn() -> Vec<PathBuf>) -> JsonProviderSpec {
    JsonProviderSpec {
        provider_id,
        extension_key: "provider_session",
        roots,
    }
}

fn home_join(relative: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(relative))
}

fn mac_app_support(app: &str) -> Option<PathBuf> {
    home_join(&format!("Library/Application Support/{app}"))
}

fn vscode_global_storage(app: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "macos")]
    if let Some(root) = mac_app_support(app) {
        roots.push(root.join("User").join("globalStorage"));
        roots.push(root.join("User").join("workspaceStorage"));
    }

    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let root = PathBuf::from(appdata).join(app);
        roots.push(root.join("User").join("globalStorage"));
        roots.push(root.join("User").join("workspaceStorage"));
    }

    #[cfg(target_os = "linux")]
    if let Some(root) = home_join(&format!(".config/{app}")) {
        roots.push(root.join("User").join("globalStorage"));
        roots.push(root.join("User").join("workspaceStorage"));
    }

    roots
}

pub struct AugmentProvider;

fn augment_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home_join(".augment") {
        roots.push(home);
    }
    roots.extend(
        vscode_global_storage("Code")
            .into_iter()
            .map(|root| root.join("augmentcode.augment")),
    );
    roots
}

impl Provider for AugmentProvider {
    fn id(&self) -> &'static str {
        "augment"
    }
    fn name(&self) -> &'static str {
        "Augment"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            ..ProviderCapabilities::default()
        }
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        generic_json::scan_sessions(spec("augment", augment_roots))
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        generic_json::import_session(spec("augment", augment_roots), source_path)
    }
}
