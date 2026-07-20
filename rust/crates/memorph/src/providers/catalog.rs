//! 统一 provider 目录：身份、显示名、能力、安装状态、分类与排序的唯一装配层。

use serde::Serialize;

use crate::agent_environment::AgentEnvironmentStatus;
use crate::provider::ProviderCapabilities;
use crate::providers::{aliases::canonical_provider_id, find_provider};

/// Filter 标签：决定 provider 是否进入某个 UI 候选集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProviderFilterTag {
    #[serde(rename = "is_installed")]
    Installed,
    #[serde(rename = "has_sessions")]
    HasSessions,
}

/// 安装状态视图（按设计文档 JSON 字段命名）。
#[derive(Debug, Clone, Serialize)]
pub struct InstallState {
    #[serde(rename = "is_installed")]
    pub is_installed: bool,
    #[serde(rename = "exec_path", skip_serializing_if = "Option::is_none")]
    pub exec_path: Option<String>,
    #[serde(rename = "exec_dir", skip_serializing_if = "Option::is_none")]
    pub exec_dir: Option<String>,
    pub config_path: String,
    pub install_method: String,
}

/// 单个 provider 的完整目录视图。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub provider_id: String,
    pub display_name: String,
    pub capability_set: ProviderCapabilities,
    pub install_state: InstallState,
    pub filter_tags: Vec<ProviderFilterTag>,
    pub hidden_state: HiddenState,
    pub sort_order: SortOrder,
    pub active_time: ActiveTime,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct HiddenState {
    pub global: bool,
    pub workspace: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SortOrder {
    /// 在全局 order 中的位置；`-1` 表示未进入。
    pub global: i64,
    /// 在当前 workspace order 中的位置；`-1` 表示未进入。
    pub workspace: i64,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self {
            global: -1,
            workspace: -1,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ActiveTime {
    /// 所有 workspace 中最近一次会话活跃毫秒时间戳；无则为 `0`。
    pub global: i64,
    /// 当前 workspace 中最近一次会话活跃毫秒时间戳；无则为 `0`。
    pub workspace: i64,
}

/// 目录数据。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderView>,
}

/// 精选显示名映射：canonical id -> 展示名。未命中时兜底到 Provider::name()。
fn curated_display_name(canonical_id: &str) -> Option<&'static str> {
    Some(match canonical_id {
        "claude" => "Claude",
        "codex" => "Codex",
        "cline" => "Cline",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        "kiro" => "Kiro",
        "kimi" => "Kimi",
        "gemini" => "Gemini",
        "deepseek" => "DeepSeek",
        "antigravity" => "Antigravity",
        "copilot" => "Copilot",
        "windsurf" => "Windsurf",
        "codebuddy" => "CodeBuddy",
        "qoder" => "Qoder",
        "trae" => "Trae",
        "droid" => "Droid",
        "workbuddy" => "WorkBuddy",
        "hermes" => "Hermes",
        "pi" => "Pi",
        _ => return None,
    })
}

pub fn display_name(id: &str) -> String {
    let canonical = canonical_provider_id(id);
    if let Some(name) = curated_display_name(&canonical) {
        return name.to_string();
    }
    find_provider(&canonical)
        .map(|provider| provider.name().to_string())
        .unwrap_or(canonical)
}

/// 构建目录所需的输入。
pub struct CatalogInput<'a> {
    /// 全局排序后的 canonical id 列表。
    pub ordered_ids: &'a [String],
    /// 全局隐藏 id 集合。
    pub hidden_global: &'a [String],
    /// 当前 workspace 隐藏 id 集合。
    pub hidden_workspace: &'a [String],
    /// provider id 是否在当前 workspace 有 >0 个会话。
    pub has_sessions: &'a dyn Fn(&str) -> bool,
    /// provider id -> 安装环境。
    pub environment: &'a dyn Fn(&str) -> AgentEnvironmentStatus,
    /// provider id -> (global_last_active_ms, workspace_last_active_ms)。
    pub active_time: &'a dyn Fn(&str) -> (i64, i64),
}

pub fn build_catalog(input: CatalogInput<'_>) -> ProviderCatalog {
    let in_list = |list: &[String], id: &str| list.iter().any(|item| item == id);

    let providers = input
        .ordered_ids
        .iter()
        .filter_map(|id| {
            let provider = find_provider(id)?;
            let environment = (input.environment)(id);
            let (global_active, workspace_active) = (input.active_time)(id);

            let mut filter_tags = Vec::new();
            if environment.installed {
                filter_tags.push(ProviderFilterTag::Installed);
            }
            if (input.has_sessions)(id) {
                filter_tags.push(ProviderFilterTag::HasSessions);
            }

            let hidden_state = HiddenState {
                global: in_list(input.hidden_global, id),
                workspace: in_list(input.hidden_workspace, id),
            };

            Some(ProviderView {
                provider_id: id.clone(),
                display_name: display_name(id),
                capability_set: provider.capabilities(),
                install_state: InstallState {
                    is_installed: environment.installed,
                    exec_path: environment.executable_path,
                    exec_dir: environment.executable_dir,
                    config_path: environment.config_path.clone(),
                    install_method: environment.install_method.clone(),
                },
                filter_tags,
                hidden_state,
                sort_order: SortOrder::default(),
                active_time: ActiveTime {
                    global: global_active,
                    workspace: workspace_active,
                },
            })
        })
        .collect();

    ProviderCatalog { providers }
}

/// 按设计文档规则对目录进行最终排序。
///
/// 排序分两段：
/// 1. 用户显式排序段：workspace order 优先于 global order。
/// 2. 计算默认段：未进入用户 order 的 provider，按
///    has_sessions -> installed -> active_time.workspace -> active_time.global -> display_name。
pub fn sort_catalog(catalog: &mut ProviderCatalog) {
    catalog.providers.sort_by(|left, right| {
        let left_in_workspace = left.sort_order.workspace >= 0;
        let right_in_workspace = right.sort_order.workspace >= 0;
        let left_in_global = left.sort_order.global >= 0;
        let right_in_global = right.sort_order.global >= 0;

        let left_explicit = if left_in_workspace {
            (0, left.sort_order.workspace as usize)
        } else if left_in_global {
            (1, left.sort_order.global as usize)
        } else {
            (2, usize::MAX)
        };
        let right_explicit = if right_in_workspace {
            (0, right.sort_order.workspace as usize)
        } else if right_in_global {
            (1, right.sort_order.global as usize)
        } else {
            (2, usize::MAX)
        };

        if left_explicit != right_explicit {
            return left_explicit.cmp(&right_explicit);
        }

        // Both are in explicit order or both are default.
        if left_explicit.0 == 2 {
            let left_has_sessions = left.filter_tags.contains(&ProviderFilterTag::HasSessions);
            let right_has_sessions = right.filter_tags.contains(&ProviderFilterTag::HasSessions);
            let left_installed = left.filter_tags.contains(&ProviderFilterTag::Installed);
            let right_installed = right.filter_tags.contains(&ProviderFilterTag::Installed);

            let default_key = (
                !left_has_sessions,
                !left_installed,
                -(left.active_time.workspace),
                -(left.active_time.global),
                left.display_name.clone(),
            );
            let right_default_key = (
                !right_has_sessions,
                !right_installed,
                -(right.active_time.workspace),
                -(right.active_time.global),
                right.display_name.clone(),
            );
            return default_key.cmp(&right_default_key);
        }

        std::cmp::Ordering::Equal
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        PageStrategy, ProviderContentFidelity, ResumeQuality, ScanStrategy, StorageShape,
        TurnQuality, WriteRiskLevel,
    };

    fn env(installed: bool) -> AgentEnvironmentStatus {
        AgentEnvironmentStatus {
            installed,
            executable_path: None,
            executable_dir: None,
            config_path: String::new(),
            install_method: String::new(),
            executable_version: None,
        }
    }

    fn active(global: i64, workspace: i64) -> (i64, i64) {
        (global, workspace)
    }

    #[test]
    fn display_name_uses_curated_map_and_canonicalizes() {
        assert_eq!(display_name("claude"), "Claude");
        assert_eq!(display_name("opencode"), "OpenCode");
        assert_eq!(display_name("deepseek"), "DeepSeek");
        assert_eq!(display_name("gemini"), "Gemini");
    }

    #[test]
    fn build_catalog_assigns_filters_and_hidden_state() {
        let ordered = vec!["claude".to_string(), "codex".to_string()];
        let hidden_global = vec!["codex".to_string()];
        let hidden_workspace: Vec<String> = Vec::new();

        let catalog = build_catalog(CatalogInput {
            ordered_ids: &ordered,
            hidden_global: &hidden_global,
            hidden_workspace: &hidden_workspace,
            has_sessions: &|_| false,
            environment: &|id| env(id == "claude"),
            active_time: &|_| active(0, 0),
        });

        let claude = catalog
            .providers
            .iter()
            .find(|p| p.provider_id == "claude")
            .unwrap();
        assert_eq!(claude.display_name, "Claude");
        assert!(claude.filter_tags.contains(&ProviderFilterTag::Installed));
        assert!(!claude.hidden_state.global);

        let codex = catalog
            .providers
            .iter()
            .find(|p| p.provider_id == "codex")
            .unwrap();
        assert!(!codex.filter_tags.contains(&ProviderFilterTag::Installed));
        assert!(codex.hidden_state.global);
    }

    #[test]
    fn mature_provider_catalogs_have_complete_quality_metadata() {
        for provider_id in ["claude", "codex", "kimi", "opencode"] {
            let capabilities = find_provider(provider_id).unwrap().capabilities();
            assert_ne!(capabilities.scan_strategy, ScanStrategy::Unknown);
            assert_ne!(capabilities.page_strategy, PageStrategy::Unknown);
            assert_ne!(capabilities.storage_shape, StorageShape::Unknown);
            assert_ne!(capabilities.turn_quality, TurnQuality::Unknown);
            assert_ne!(capabilities.resume_quality, ResumeQuality::None);
            assert_ne!(capabilities.write_risk.level, WriteRiskLevel::Unknown);
            assert!(fidelity_is_complete(capabilities.import_fidelity));
            assert!(fidelity_is_complete(capabilities.export_fidelity));
            assert!(capabilities.activity_support.hook_events);
            assert!(capabilities.activity_support.runtime_endpoint);
            assert!(capabilities.activity_support.session_activity);
            assert!(capabilities.scan);
            assert!(capabilities.import);
            assert!(capabilities.export);
            assert!(capabilities.delete);
            assert!(capabilities.rename);
            assert!(capabilities.resume);
        }
    }

    #[test]
    fn sort_catalog_puts_workspace_order_first() {
        let mut catalog = build_catalog(CatalogInput {
            ordered_ids: &[
                "claude".to_string(),
                "codex".to_string(),
                "opencode".to_string(),
            ],
            hidden_global: &[],
            hidden_workspace: &[],
            has_sessions: &|_| false,
            environment: &|_| env(true),
            active_time: &|_| active(0, 0),
        });

        // global order: claude(0), codex(1), opencode(2)
        catalog.providers[0].sort_order.global = 0;
        catalog.providers[1].sort_order.global = 1;
        catalog.providers[2].sort_order.global = 2;

        // workspace order: opencode(0), claude(1)
        catalog.providers[0].sort_order.workspace = 1; // claude
        catalog.providers[2].sort_order.workspace = 0; // opencode

        sort_catalog(&mut catalog);
        let ids: Vec<_> = catalog
            .providers
            .iter()
            .map(|p| p.provider_id.clone())
            .collect();
        assert_eq!(ids, vec!["opencode", "claude", "codex"]);
    }

    #[test]
    fn sort_catalog_default_segment_orders_by_signals() {
        let mut catalog = build_catalog(CatalogInput {
            ordered_ids: &[
                "claude".to_string(),
                "codex".to_string(),
                "opencode".to_string(),
            ],
            hidden_global: &[],
            hidden_workspace: &[],
            has_sessions: &|id| id == "claude" || id == "opencode",
            environment: &|id| env(id == "claude" || id == "opencode"),
            active_time: &|id| match id {
                "claude" => active(100, 100),
                "opencode" => active(200, 200),
                _ => active(0, 0),
            },
        });

        sort_catalog(&mut catalog);
        let ids: Vec<_> = catalog
            .providers
            .iter()
            .map(|p| p.provider_id.clone())
            .collect();
        // Both have sessions; opencode is more recent; both installed; codex is neither.
        assert_eq!(ids, vec!["opencode", "claude", "codex"]);
    }

    fn fidelity_is_complete(fidelity: ProviderContentFidelity) -> bool {
        [
            fidelity.text,
            fidelity.thinking,
            fidelity.tool_call,
            fidelity.tool_result,
            fidelity.patch,
            fidelity.image,
            fidelity.file,
            fidelity.compressed,
            fidelity.provider_payload,
        ]
        .into_iter()
        .all(|value| value.is_some())
    }
}
