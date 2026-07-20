use crate::canonical::ImportedSession;
use crate::provider::{Provider, ProviderCapabilities, ProviderSessionSummary};
use crate::providers::generic_json::{self, JsonProviderSpec};
use anyhow::Result;
use std::path::PathBuf;

macro_rules! json_read_provider {
    ($struct_name:ident, $provider_id:literal, $name:literal, $roots:ident, $resume:expr) => {
        pub struct $struct_name;

        impl Provider for $struct_name {
            fn id(&self) -> &'static str {
                $provider_id
            }

            fn name(&self) -> &'static str {
                $name
            }

            fn capabilities(&self) -> ProviderCapabilities {
                ProviderCapabilities {
                    scan: true,
                    import: true,
                    export: false,
                    delete: false,
                    rename: false,
                    resume: $resume.is_some(),
                    ..ProviderCapabilities::default()
                }
            }

            fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
                generic_json::scan_sessions(spec($provider_id, $roots))
            }

            fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
                generic_json::import_session(spec($provider_id, $roots), source_path)
            }

            fn resume_command(&self, session_id: &str) -> Option<String> {
                $resume.map(|command: &'static str| format!("{} {}", command, session_id))
            }
        }
    };
}

json_read_provider!(
    AntigravityProvider,
    "antigravity",
    "Antigravity",
    antigravity_roots,
    None::<&'static str>
);
json_read_provider!(
    WindsurfProvider,
    "windsurf",
    "Windsurf",
    windsurf_roots,
    None::<&'static str>
);
json_read_provider!(
    QwenProvider,
    "qwen",
    "Qwen Code",
    qwen_roots,
    None::<&'static str>
);
json_read_provider!(
    StepFunProvider,
    "stepfun",
    "StepFun",
    stepfun_roots,
    None::<&'static str>
);
json_read_provider!(
    TraeProvider,
    "trae",
    "TraeCli",
    trae_roots,
    None::<&'static str>
);

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

fn antigravity_roots() -> Vec<PathBuf> {
    // Antigravity (Google) reuses the Gemini CLI backend for conversations.
    // CodeIsland reference: ~/.gemini/tmp/<project_hash>/chats/*.json
    let mut roots = Vec::new();
    if let Some(root) = home_join(".gemini/tmp") {
        roots.push(root);
    }
    let mut vscode = vscode_global_storage("Antigravity");
    vscode.extend(vscode_global_storage("Google/Antigravity"));
    roots.extend(vscode);
    if let Some(root) = home_join(".antigravity") {
        roots.push(root);
    }
    roots
}

fn windsurf_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Windsurf");
    if let Some(root) = home_join(".codeium/windsurf") {
        roots.push(root);
    }
    roots
}

fn qwen_roots() -> Vec<PathBuf> {
    // Qwen Code CLI stores Claude-style transcripts per project.
    // local-llm-proxy reference: ~/.qwen/projects/<encoded-cwd>/chats/*.jsonl
    let mut roots = Vec::new();
    if let Some(root) = home_join(".qwen/projects") {
        roots.push(root);
    }
    roots.extend(vscode_global_storage("Qwen"));
    roots
}

fn stepfun_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("StepFun");
    if let Some(root) = home_join(".stepfun") {
        roots.push(root);
    }
    roots
}

fn trae_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Trae");
    if let Some(root) = home_join(".trae") {
        roots.push(root);
    }
    roots
}
