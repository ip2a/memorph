mod aliases;
pub mod antigravity;
pub mod catalog;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod deepseek;
pub mod droid;
pub mod emerging;
pub(crate) mod environment_profiles;
pub mod gemini;
pub(crate) mod generic_json;
pub mod hermes;
pub mod kimi;
pub mod kiro;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod qoder;
pub mod qwen;
pub mod trae;
pub mod windsurf;
pub mod workbuddy;

pub mod amazonq;
pub(crate) mod hook_profiles;
mod hook_registry;
pub(crate) mod q_conversation;

use crate::provider::Provider;

const PROVIDER_IDS: &[&str] = &[
    "claude",
    "codex",
    "cline",
    "cursor",
    "opencode",
    "openclaw",
    "augment",
    "kiro",
    "deepseek",
    "kimi",
    "gemini",
    "antigravity",
    "copilot",
    "windsurf",
    "codebuddy",
    "qoder",
    "trae",
    "droid",
    "workbuddy",
    "hermes",
    "amazonq",
    "qwen",
    "pi",
];

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn ids() -> &'static [&'static str] {
        PROVIDER_IDS
    }

    pub fn find(id: &str) -> Option<Box<dyn Provider>> {
        let id = aliases::canonical_provider_id(id);
        match id.as_str() {
            "claude" => Some(Box::new(claude::ClaudeProvider)),
            "codex" => Some(Box::new(codex::CodexProvider)),
            "cline" => Some(Box::new(cline::ClineProvider)),
            "cursor" => Some(Box::new(cursor::CursorProvider)),
            "deepseek" => Some(Box::new(deepseek::DeepseekProvider)),
            "antigravity" => Some(Box::new(antigravity::AntigravityProvider)),
            "copilot" => Some(Box::new(copilot::CopilotProvider)),
            "windsurf" => Some(Box::new(windsurf::WindsurfProvider)),
            "codebuddy" => Some(Box::new(codebuddy::CodeBuddyProvider)),
            "qoder" => Some(Box::new(qoder::QoderProvider)),
            "trae" => Some(Box::new(trae::TraeProvider)),
            "droid" => Some(Box::new(droid::DroidProvider)),
            "workbuddy" => Some(Box::new(workbuddy::WorkBuddyProvider)),
            "hermes" => Some(Box::new(hermes::HermesProvider)),
            "amazonq" => Some(Box::new(amazonq::AmazonQProvider)),
            "qwen" => Some(Box::new(qwen::QwenProvider)),
            "pi" => Some(Box::new(pi::PiProvider)),
            "gemini" => Some(Box::new(gemini::GeminiProvider)),
            "kiro" => Some(Box::new(kiro::KiroProvider)),
            "kimi" => Some(Box::new(kimi::KimiProvider)),
            "opencode" => Some(Box::new(opencode::OpenCodeProvider)),
            "openclaw" => Some(Box::new(openclaw::OpenClawProvider)),
            "augment" => Some(Box::new(emerging::AugmentProvider)),
            _ => None,
        }
    }
}

pub fn all_provider_ids() -> &'static [&'static str] {
    ProviderRegistry::ids()
}

/// Find provider by ID.
pub fn find_provider(id: &str) -> Option<Box<dyn Provider>> {
    ProviderRegistry::find(id)
}

pub(crate) fn canonical_provider_id(provider_id: &str) -> String {
    aliases::canonical_provider_id(provider_id)
}

/// Find provider-owned hook management implementation by ID or hook profile alias.
pub fn find_provider_hook(
    provider: &str,
) -> Option<&'static dyn crate::hooks::contract::ProviderHook> {
    hook_registry::find_provider_hook(provider)
}

/// Find provider-owned hook payload adapter by ID or hook profile alias.
pub fn find_hook_adapter(
    provider: &str,
) -> Option<&'static dyn crate::hooks::contract::HookAdapter> {
    hook_registry::find_hook_adapter(provider)
}

/// Default switch target for a given source provider.
pub fn default_switch_target(from: &str) -> &'static str {
    let ids = all_provider_ids();
    match from {
        "codex" => "claude",
        _ => ids
            .iter()
            .find(|&&id| id != from)
            .copied()
            .unwrap_or("codex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Fidelity;

    #[test]
    fn registry_exposes_requested_emerging_providers() {
        for id in [
            "gemini",
            "antigravity",
            "cline",
            "copilot",
            "windsurf",
            "codebuddy",
            "qoder",
            "trae",
            "droid",
            "workbuddy",
            "hermes",
            "pi",
            "amazonq",
            "qwen",
            "openclaw",
            "augment",
        ] {
            assert!(
                all_provider_ids().contains(&id),
                "missing provider id: {id}"
            );
            assert!(find_provider(id).is_some(), "provider not found: {id}");
        }
    }

    #[test]
    fn registry_uses_native_hermes_sqlite_provider() {
        let provider = find_provider("hermes").expect("hermes provider");
        let capabilities = provider.capabilities();
        assert_eq!(
            capabilities.storage_shape,
            crate::provider::StorageShape::Sqlite
        );
        assert_eq!(
            capabilities.page_strategy,
            crate::provider::PageStrategy::FullImport
        );
        assert!(capabilities.resume);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.export);
    }

    #[test]
    fn factory_alias_resolves_to_droid_provider() {
        let provider = find_provider("factory").expect("factory alias");
        assert_eq!(provider.id(), "droid");
    }

    /// Providers without export must not claim meaningful export fidelity:
    /// if export=false, every export_fidelity field must be None or Unsupported.
    /// Providers with export must declare all 9 export_fidelity fields.
    #[test]
    fn all_providers_declare_consistent_fidelity() {
        fn all_fields(f: &crate::provider::ProviderContentFidelity) -> [Option<Fidelity>; 9] {
            [
                f.text,
                f.thinking,
                f.tool_call,
                f.tool_result,
                f.patch,
                f.image,
                f.file,
                f.compressed,
                f.provider_payload,
            ]
        }

        for id in all_provider_ids() {
            let provider = find_provider(id).unwrap_or_else(|| panic!("provider {id}"));
            let caps = provider.capabilities();

            let export_fields = all_fields(&caps.export_fidelity);
            if caps.export {
                for (i, field) in export_fields.iter().enumerate() {
                    assert!(
                        field.is_some(),
                        "{id}: export=true but export_fidelity field {i} is None",
                    );
                }
            } else {
                for (i, field) in export_fields.iter().enumerate() {
                    assert!(
                        matches!(field, None | Some(Fidelity::Unsupported)),
                        "{id}: export=false but export_fidelity field {i} claims {field:?}",
                    );
                }
            }
        }
    }

    /// For every export-capable provider, export_report must produce
    /// issues that exactly match the declared export_fidelity dispositions.
    #[test]
    fn export_report_matches_declared_fidelity_for_all_exporters() {
        use crate::provider::export_report;
        use crate::session::{Block, Context, EventKind, Identity, Links, Metadata, Role, Schema};
        use chrono::Utc;

        let now = Utc::now();
        let blocks = vec![
            Block::Text { text: "t".into() },
            Block::Thinking {
                text: "th".into(),
                signature: None,
            },
            Block::ToolCall {
                tool_call_id: "tc1".into(),
                name: "tool".into(),
                input: None,
            },
            Block::ToolResult {
                tool_call_id: "tc1".into(),
                content: "r".into(),
                outcome: crate::session::execution_outcome(false),
            },
            Block::Patch {
                summary: None,
                diff_text: Some(
                    "--- a
+++ b"
                        .into(),
                ),
                files: vec!["f".into()],
                hash: None,
            },
            Block::Image {
                mime_type: "image/png".into(),
                data: Some("d".into()),
                path: None,
            },
            Block::File {
                path: "f".into(),
                content: Some("c".into()),
                mime_type: None,
            },
            Block::Compressed {
                raw: serde_json::json!({"summary": "s"}),
            },
            Block::Other {
                raw: serde_json::json!({"custom": "payload"}),
            },
        ];

        let session = crate::session::Session {
            lineage: Vec::new(),
            schema: Schema {
                name: crate::session::OASF_SCHEMA_NAME.into(),
                version: crate::session::OASF_SCHEMA_VERSION,
            },
            identity: Identity {
                id: "s1".into(),
                title: None,
            },
            context: Context::default(),
            events: vec![crate::session::Event {
                id: "e1".into(),
                kind: EventKind::Message,
                role: Role::Assistant,
                timestamp: now,
                links: Links::default(),
                blocks,
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: None,
                    usage: None,
                },
            }],
            extensions: Default::default(),
        };

        for id in all_provider_ids() {
            let provider = find_provider(id).unwrap();
            let caps = provider.capabilities();
            if !caps.export {
                continue;
            }

            let report = export_report(id, &session, caps);

            let ef = caps.export_fidelity;
            let mut expected_issues = 0u32;
            // Preserved → no issue; any other non-None → one issue
            for (field, kind) in [
                (ef.text, "text"),
                (ef.thinking, "thinking"),
                (ef.tool_call, "tool_call"),
                (ef.tool_result, "tool_result"),
                (ef.patch, "patch"),
                (ef.image, "image"),
                (ef.file, "file"),
                (ef.compressed, "compressed"),
                (ef.provider_payload, "other"),
            ] {
                if matches!(field, Some(Fidelity::Preserved) | None) {
                    continue;
                }
                expected_issues += 1;
                // Verify the issue code exists in the report
                let expected_code = format!(
                    "{kind}_export_{}",
                    match field.unwrap() {
                        Fidelity::Normalized => "normalized",
                        Fidelity::Downgraded => "downgraded",
                        Fidelity::Dropped => "dropped",
                        Fidelity::Unsupported => "unsupported",
                        Fidelity::Preserved => "preserved",
                    }
                );
                assert!(
                    report.issues.iter().any(|i| i.code == expected_code),
                    "{id}: expected report issue `{expected_code}` not found"
                );
            }
            assert_eq!(
                report.issues.len(),
                expected_issues as usize,
                "{id}: report has {} issues but expected {expected_issues}",
                report.issues.len()
            );
        }
    }
}
