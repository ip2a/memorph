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
    DroidProvider,
    "droid",
    "Factory",
    droid_roots,
    None::<&'static str>
);
json_read_provider!(
    ClineProvider,
    "cline",
    "Cline",
    cline_roots,
    None::<&'static str>
);
json_read_provider!(
    CopilotProvider,
    "copilot",
    "GitHub Copilot",
    copilot_roots,
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
    CideBuddyProvider,
    "cidebuddy",
    "CodeBuddy",
    cidebuddy_roots,
    None::<&'static str>
);
json_read_provider!(
    CodeBuddyProvider,
    "codebuddy",
    "CodeBuddy",
    codebuddy_roots,
    None::<&'static str>
);
json_read_provider!(
    CodyBuddyCnProvider,
    "codybuddycn",
    "CodyBuddyCN",
    codybuddycn_roots,
    None::<&'static str>
);
json_read_provider!(
    QoderProvider,
    "qoder",
    "Qoder",
    qoder_roots,
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
    WorkBuddyProvider,
    "workbuddy",
    "WorkBuddy",
    workbuddy_roots,
    None::<&'static str>
);
json_read_provider!(
    HermesProvider,
    "hermes",
    "Hermes",
    hermes_roots,
    None::<&'static str>
);
json_read_provider!(PiProvider, "pi", "pi", pi_roots, None::<&'static str>);
json_read_provider!(
    OmpProvider,
    "omp",
    "Oh My Pi",
    omp_roots,
    None::<&'static str>
);
json_read_provider!(
    TraeProvider,
    "trae",
    "TraeCli",
    trae_roots,
    None::<&'static str>
);
json_read_provider!(
    TraeGuiProvider,
    "trae_gui",
    "Trae",
    trae_roots,
    None::<&'static str>
);
json_read_provider!(
    TraeCnProvider,
    "traecn",
    "Trae CN",
    traecn_roots,
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
    let mut roots = vscode_global_storage("Antigravity");
    roots.extend(vscode_global_storage("Google/Antigravity"));
    if let Some(root) = home_join(".antigravity") {
        roots.push(root);
    }
    roots
}

fn droid_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Factory");
    roots.extend(vscode_global_storage("Droid"));
    if let Some(root) = home_join(".factory") {
        roots.push(root);
    }
    roots
}

fn cline_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Code");
    roots.extend(vscode_global_storage("Code - Insiders"));
    roots.extend(vscode_global_storage("VSCodium"));
    if let Some(root) = home_join("Documents/Cline") {
        roots.push(root);
    }
    roots
}

fn copilot_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for app in ["Code", "Code - Insiders", "VSCodium"] {
        for root in vscode_global_storage(app) {
            roots.push(root.join("github.copilot-chat"));
            roots.push(root.join("github.copilot"));
        }
    }
    if let Some(root) = home_join(".copilot") {
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

fn cidebuddy_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("CodeBuddy");
    roots.extend(vscode_global_storage("CideBuddy"));
    if let Some(root) = home_join(".codebuddy") {
        roots.push(root);
    }
    if let Some(root) = home_join(".cidebuddy") {
        roots.push(root);
    }
    roots
}

fn codebuddy_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("CodeBuddy");
    if let Some(root) = home_join(".codebuddy") {
        roots.push(root);
    }
    roots
}

fn codybuddycn_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("CodyBuddyCN");
    if let Some(root) = home_join(".codybuddycn") {
        roots.push(root);
    }
    roots
}

fn qoder_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Qoder");
    if let Some(root) = home_join(".qoder") {
        roots.push(root);
    }
    roots
}

fn qwen_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Qwen");
    if let Some(root) = home_join(".qwen") {
        roots.push(root);
    }
    roots
}

fn stepfun_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("StepFun");
    if let Some(root) = home_join(".stepfun") {
        roots.push(root);
    }
    roots
}

fn workbuddy_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("WorkBuddy");
    if let Some(root) = home_join(".workbuddy") {
        roots.push(root);
    }
    roots
}

fn hermes_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Hermes");
    if let Some(root) = home_join(".hermes") {
        roots.push(root);
    }
    roots
}

fn pi_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = home_join(".pi/agent") {
        roots.push(root);
    }
    roots
}

fn omp_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = home_join(".omp/agent") {
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

fn traecn_roots() -> Vec<PathBuf> {
    let mut roots = vscode_global_storage("Trae CN");
    if let Some(root) = home_join(".trae-cn") {
        roots.push(root);
    }
    roots
}
