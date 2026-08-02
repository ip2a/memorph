# 技术文档 — memorph 核心 lib 说明 v1

> 本文说明 `memorph` 作为 Rust 库(`cargo add memorph`)对外提供的能力。
> 核心 lib 是纯领域逻辑 + 数据/文件 IO,**不含** CLI/TUI/Web server(axum)/桌面 GUI —— 那些在 `memorph-cli` crate。
> 所有跨 provider 的会话操作都以 **canonical 中间格式** 为枢纽,不直接 provider-to-provider。

---

## 1. 定位

memorph 核心 lib 把"AI coding 会话"在 **不同 Agent provider(Claude Code / Codex / Cursor / OpenCode / Gemini / Kimi / Kiro / Windsurf / Cline / Copilot / DeepSeek / Trae … 共 20+ )** 之间:

- **扫描 / 查询**(按 provider / workspace / 时间 / 内容)
- **转换 / 导入 / 导出 / 切换**(provider ↔ canonical ↔ provider)
- **变更**(删除 / 重命名 / 状态更新)
- **同步**(多 provider 副本组成 sync group,双向推送)
- **压缩**(长会话归档为 `memorph-archive://` 引用,按需检索/还原)
- **备份 / 恢复**(操作前自动备份、数据库 bundle 备份/恢复)
- **检索**(事件级全文匹配)
- **观测**(会话统计、活动流水、artifact 管理)

所有操作都记录到本地 SQLite,以 **ActivityActor**(Cli / Api / System)标记来源,便于审计。

---

## 2. 设计基石:canonical 中间格式

`canonical` 模块定义与 provider 无关的会话数据模型(`memorph-canonical` schema,version 2)。每个 provider 适配器负责 ↔ canonical 的双向映射。

核心类型(`memorph::canonical`):

| 类型 | 含义 |
|---|---|
| `CanonicalSession` | 一次会话的规范表示(身份 + 上下文 + 事件流 + 元数据) |
| `SessionIdentity` / `SessionProvenance` / `ProviderSessionRef` | 会话身份与来源追溯 |
| `SessionContext` / `WorkspaceSessionState` | 会话上下文(workspace / cwd / agent) |
| `SessionEvent` / `SessionEventKind` / `EventRole` / `EventBlock` | 事件流(用户消息 / 思考 / 工具调用 / 工具结果 / 补丁 / 图片 / 文件 / 压缩段 …) |
| `EventLinks` / `TurnBoundary` / `EventMetadata` / `EventSource` | 事件间关系、轮次边界、来源 |
| `UsageStats` | token / 字节用量 |
| `SessionArtifact` / `ArtifactKind` | 会话产物 |
| `LocalSessionState` / `SessionLocator` | 本地状态与定位 |

`SessionEvent` 是事件级最小单元;`CanonicalSession` 是聚合。所有跨 provider 操作(list / switch / compress / search)都在 `CanonicalSession`/`SessionEvent` 之上工作。

---

## 3. Provider 体系

### 3.1 Provider trait 与能力自描述

`memorph::provider` 定义:

- `trait Provider`:每个 provider 适配器实现它,提供 scan / import / export / delete / rename / resume 等能力。
- `ProviderCapabilities`:**自描述能力矩阵**,运行时可查询,无需硬编码"哪个 provider 支持什么":
  - `ScanStrategy` / `PageStrategy` / `StorageShape`(发现与分页策略)
  - `TurnQuality` / `ResumeQuality`(轮次推断与恢复质量)
  - `ProviderWriteRisk` + `WriteRiskLevel`(写风险等级:决定是否强制备份)
  - `ProviderBackupSupport`(是否 before_write / restore / sync_only)
  - `ProviderActivitySupport`(hook 事件 / runtime endpoint / 会话活动)
  - `ProviderContentFidelity`(text / thinking / tool_call / tool_result / patch / image / file / compressed / provider_payload 的保真度:preserved / normalized / downgraded / dropped / unsupported)
  - `ProviderCompressionSupport` / `CompressionProjection`(压缩支持与投影策略)
- `ProviderSessionSummary` / `ProviderSourceFingerprint` / `ProviderSessionImportPage` / `ProviderSessionBackup` / `ProviderSourceMutation`:扫描/导入/备份的数据载体。

### 3.2 ProviderRegistry

`memorph::providers::ProviderRegistry`:
- `ProviderRegistry::ids()` / `find(id)` / `all_provider_ids()`
- 通过 canonical provider ID 或别名(`providers::aliases`)解析到 `Box<dyn Provider>`
- `providers::catalog`:display name / 能力目录

### 3.3 内置 provider

`memorph::providers::<id>`:claude、codex、opencode、cursor、gemini、kimi、kiro、windsurf、cline、copilot、deepseek、droid、antigravity、codebuddy、copilot、codybuddycn、hermes、omp、pi、qoder、qwen、stepfun、trae/trae_gui/traecn、workbuddy 等(小型 provider 共享 `emerging.rs`)。

---

## 4. 核心能力(orchestrator:`memorph::core`)

`core` 是操作编排层,按职责拆为 14 个子模块。所有操作以 `Result` 返回,变更类操作以 `ActivityActor` 记录来源。

### 4.1 会话查询 — `core::projection` / `core::query` / `core::sessions`

```rust
// 列出会话(分页 / 排序 / workspace 过滤 / 消息计数 / hook 过滤)
core::projection::list_sessions(&SessionListParams { all, providers, cwd, limit, offset, sort, .. }) -> Vec<SessionGroup>

// 按目录 / session_id / provider 查找
core::query::find_sessions(&FindParams { dir, session, providers }) -> Vec<SessionGroup>

// 会话详情(含 canonical 事件流、投影报告、活动、artifact)
core::sessions::get_session_detail_view(provider_id, session_id) -> SessionDetailView
core::sessions::get_session_detail_view_page(provider_id, session_id, event_offset, event_limit) -> SessionDetailView
core::sessions::get_canonical_session(provider_id, session_id) -> ImportedSession

// 投影维护(把 provider 原生会话投影进 SQLite 索引)
core::projection::bootstrap_session_projections(provider_filter, actor) -> BootstrapReport
core::projection::refresh_projected_session_staleness(actor) -> StaleRefreshReport
core::projection::reproject_stale_sessions(provider_filter, actor) -> ReprojectReport
```

### 4.2 会话转换 — `core::transfer`

```rust
core::transfer::export_session(&ExportParams { provider, session_id, output_prefix, output_dir, format }, actor) -> ExportResult
core::transfer::import_session(&ImportParams { provider, file_or_id, to_dir }, actor) -> ImportResult
core::transfer::switch_session(&SwitchParams { from, to, session_id, to_dir, target_title, move_original }) -> SwitchResult
```

`format` 支持 `.json` / `.md` / `.html` / `.morph`(见 `format` 模块)。

### 4.3 会话变更 — `core::session_mutation`

```rust
core::session_mutation::delete_session(provider_id, session_id, actor) -> ()
core::session_mutation::delete_sessions(provider_id, &[session_ids], actor) -> DeleteReport
core::session_mutation::rename_session(provider_id, session_id, new_title, actor) -> RenameResult
core::session_mutation::update_session_local_state(...) -> ...
```

### 4.4 会话管理与备份 — `core::session_management`

```rust
// workspace 归一化与匹配
session_management::normalized_workspace_key(provider_id, workspace) -> Option<String>
session_management::workspace_matches(provider_id, session_workspace, query_workspace) -> bool
session_management::resolve_existing_target_dir(provider_id, input) -> PathBuf

// 操作前自动注册的备份(高写风险 provider 写入前自动产生)
session_management::list_registered_backups(BackupQuery { .. }) -> Vec<BackupView>
session_management::get_registered_backup(backup_id) -> Option<BackupView>
session_management::restore_registered_backup(backup_id, actor) -> RestoreRecord
```

### 4.5 同步组 — `memorph::sync`

把同一会话在多个 provider 的副本组成 sync group,双向推送:

```rust
sync::create_group(&SyncCreateParams { provider, session_id, targets, to_dir, title }) -> SyncGroup
sync::add_holding(&AddHoldingParams { group_id, provider, session_id, to_dir }) -> Holding
sync::remove_holding(group_id, holding_id) -> ()
sync::push_sync(group_id, source_holding_id, actor) -> SyncReport
sync::sync_to_latest(group_id, actor) -> SyncReport   // 以最新活跃副本为源
sync::list_groups() / load_group(id) / rename_group / delete_group / refresh_active_times
```

### 4.6 压缩 — `core::compression_application` / `core::active_compression`

把长会话中可压缩的事件段归档为 `memorph-archive://` 引用,减少上下文体积,按需检索/还原:

```rust
// 应用层
compression_application::list_compression_archives(filter) -> Vec<ArchiveSummary>
compression_application::restore_compression_archive(&RestoreParams { archive_ref, .. }, actor) -> RestoreResult
compression_application::restore_native_compression(&RestoreNativeParams { .. }, actor) -> NativeRestoreResult
compression_application::retrieve_compression_archive(&RetrieveParams { archive_ref, query, max_results }) -> RetrieveResult  // 语义检索
compression_application::expand_compression_session(&ExpandParams { .. }, actor) -> ExpandResult
compression_application::compression_retrieval_tool_spec() -> CompressionRetrievalToolSpec  // 供 agent 工具描述
compression_application::active_compression_dry_run(&ActiveCompressionDryRunParams { .. }) -> ActiveCompressionReport
compression_application::active_compression_apply(&ActiveCompressionApplyCommandParams { .. }, actor) -> ActiveCompressionApplyResult

// 策略层
active_compression::ActiveCompressionPolicy { mode, protect_recent_message_events, min_candidate_bytes, min_savings_ratio_percent }
active_compression::plan_compression_candidates(..) / build_dry_run_report(..) / apply_active_compression(..)
active_compression::estimate_session_bytes(session) -> usize
```

### 4.7 备份 / 数据库 / Artifact — `core::database_management` / `core::management`

```rust
// 整库 SQLite bundle 备份/校验/恢复
database_management::backup_database(output_dir, actor) -> DatabaseBackupReport
database_management::verify_database_backup(bundle_path) -> VerifiedDatabaseBackup
database_management::restore_database(bundle_path, actor) -> DatabaseRestoreReport

// artifact(产物文件)登记与清理
management::inspect_artifacts() -> ArtifactInspectionReport   // 注册的 + orphan
management::cleanup_artifacts(retention_hours, apply, actor) -> ArtifactCleanupReport
management::list_management_activity(&ActivityQuery) -> Vec<ActivityRecord>  // 活动流水
```

### 4.8 事件检索 — `core::session_event_search`

```rust
session_event_search::session_event_matches_query(event, query) -> bool
session_event_search::find_matching_event_indices(events, query) -> Vec<usize>
```

### 4.9 管理器视图 — `core::manager`

```rust
manager::preview(&ManagerFilter) -> ManagerPreviewResult
manager::stats(&ManagerFilter) -> ManagerStatsResult
manager::clean(&[ManagerItem], actor) -> ManagerCleanResult
manager::workspaces(&ManagerFilter) / workspaces_with_sessions(..)
manager::ManagerItem::action_identity(provider_id, session_id) -> String  // 稳定操作 ID
manager::invalidate_stats_cache()
```

---

## 5. 支撑子系统

| 模块 | 能力 |
|---|---|
| `storage` | `activity_store`(活动流水 + `ActivityActor`/`ActivityQuery`/`ActivityStatus`)、`artifact_store`(`BackupQuery`/`BackupRestoreStatus` + artifact 登记)、`session_state`(`ResolvedLocalSessionState` 等本地状态)、`session_index_store`、`local_store`(`LocalSqliteStore` SQLite 句柄)、`database_backup` |
| `cache` | 进程本地缓存(`store` / `provider_sessions` / `catalog` / `compression_archives` / `manager_stats` / `agent_environment`)+ 文件系统 watcher(`init_watcher`),磁盘变更自动失效 |
| `hooks` | hook 全套:**协议**(`protocol`)、**契约**(`contract`)、**生命周期**(`lifecycle`:pid 存活检测、清理)、**运行时状态**(`runtime_state`:进程内会话快照 / endpoint 发布,**供核心与 HTTP handler 共享**)、**发现 / 安装 / 配置格式 / 桥接**(`discovery`/`operations`/`config_formats`/`bridge`)、**身份 / 规范化 / 注册表 / 存储 / 策略 / 健康 / 共享** 等。`augmentation` / `correlation` / `diagnostics` 已移出。`test_support` 在 `test-support` feature 下 |
| `skills` | skills 扫描与检视:**类型层**(`inspection`:`SkillEntry`/`SkillStatistics`/`SkillInstallation`/`SkillAsset`/`SkillDetail`/`SkillsOverview` + `inspect_bundle`/`read_frontmatter`)、**扫描**(`scanner`)、**冲突 / 覆盖 / 图 / 健康 / 调用 / 清理 / 仓储 / 上下文**(`conflicts`/`coverage`/`graph`/`health`/`invocation`/`prune`/`repository`/`context`) |
| `agent_management` / `agent_environment` | AI agent 绑定 / 状态 / push / pull、运行时环境探测 |
| `stats_dashboard` | 统计看板数据聚合(`dashboard(&query)`) |
| `session_projection` | 投影报告类型(`ProjectionFidelity`/`ProjectionItemScope`/`ProjectionOperationKind`/`ProjectionStatus` 等) |
| `config` | 用户偏好(`server_preferences`、端口常量 `DEFAULT_WEB_PORT`/`DEFAULT_API_PORT`)、`effective_home_dir`、`UiLanguage`/`LogPreferences`/`HomeButtonConfig`/`ServerPreferences`/`DesktopWindowState` 等 |
| `format` | `.json` / `.md` / `.html` / `.morph` 序列化与导入 |
| `i18n` | 共享国际化字符串(嵌入 `assets/i18n.json`) |
| `runtime` | `run_blocking(task)`:把同步任务丢到 `spawn_blocking` 线程池;启用 `test-support` feature 时透传测试 home 隔离 |
| `logging` | tracing 初始化辅助 |
| `utils` | 通用工具 |

---

## 6. 典型用法

### 6.1 列出当前 workspace 的会话

```rust
use memorph::core::{projection::{list_sessions, SessionListParams, SessionListSort}};

let groups = list_sessions(&SessionListParams {
    all: false,
    providers: vec![],
    cwd: Some(std::env::current_dir()?.to_string_lossy().to_string()),
    include_message_counts: true,
    limit: Some(50),
    offset: None,
    sort: SessionListSort::Recent,
})?;
```

> **hook 状态不混入 list**:`list_sessions` 只负责返回会话本身,**不计算 hook 安装状态** —— 避免每次 list 都对每个 provider 读 hook 配置文件(`hooks::operations::status`)的磁盘 IO。需要 hook 状态时由 agent capability / hook 管理入口单独展示,而不是混入 session 列表。因此 `SessionListParams` 不含 hook 过滤参数,`SessionItem` 不含 hook 字段。

### 6.2 把一个 Claude Code 会话切换到 Codex

```rust
use memorph::core::transfer::{switch_session, SwitchParams};

let result = switch_session(&SwitchParams {
    from: "claude".into(),
    to: "codex".into(),
    session_id: "abc123".into(),
    to_dir: None,
    target_title: None,
    move_original: false,
})?;
println!("{} -> {}", result.from_name, result.to_name);
```

### 6.3 查询某 provider 的能力

```rust
use memorph::providers;

let provider = providers::find_provider("codex")?;
let caps = provider.capabilities();
println!("scan={:?} write_risk={:?} resume={:?}",
    caps.scan_strategy, caps.write_risk.level, caps.resume_quality);
```

### 6.4 检索压缩归档(语义检索)

```rust
use memorph::core::compression_application::{retrieve_compression_archive, RetrieveCompressionArchiveParams};

let result = retrieve_compression_archive(&RetrieveCompressionArchiveParams {
    archive_ref: "memorph-archive://...".into(),
    query: "how did we fix the auth bug".into(),
    max_results: 5,
})?;
```

### 6.5 跨 provider 同步组

```rust
use memorph::sync::{create_group, SyncCreateParams};
use memorph::storage::activity_store::ActivityActor;

let group = create_group(&SyncCreateParams {
    provider: "claude".into(),
    session_id: "abc123".into(),
    targets: vec!["codex".into(), "opencode".into()],
    to_dir: None,
    title: Some("shared session".into()),
})?;
// 之后:sync::push_sync(&group.id, source_holding_id, ActivityActor::System)
```

---

## 7. 模块索引(`memorph::`)

| 模块 | 角色 |
|---|---|
| `canonical` | 会话规范数据模型(20 类型) |
| `provider` | Provider trait + 能力自描述(18 类型) |
| `providers` | ProviderRegistry + 20+ 内置 provider 适配器 |
| `core` | orchestrator:14 子模块(projection / query / sessions / transfer / session_mutation / session_management / management / compression_application / active_compression / compression / database_management / session_event_search / manager) |
| `sync` | 跨 provider 同步组 |
| `storage` | SQLite 状态层(activity / artifact / session_state / index / local_store / database_backup) |
| `cache` | 进程缓存 + fs watcher |
| `hooks` | hook 协议 / 生命周期 / 运行时状态 / 增强 / 诊断 … |
| `skills` | skills 扫描 / 检视 / 冲突 / 覆盖 / 图 / 健康 |
| `agent_management` / `agent_environment` | agent 绑定与运行时环境 |
| `session_projection` | 投影报告类型 |
| `stats_dashboard` | 统计看板 |
| `config` | 偏好与常量 |
| `format` | 序列化(.json/.md/.html/.morph) |
| `i18n` | 国际化 |
| `runtime` | `run_blocking` 工具 |
| `logging` / `utils` | 辅助 |

---

## 8. 版本与稳定性

- 核心 lib 是对外公开契约(crates.io `cargo add memorph`)。
- schema:`memorph-canonical` v2。
- 跨小版本保持 API 兼容;不兼容变更走主版本号。
- `test-support` feature 下的符号(`test_home_dir`/`reset_for_tests`/`test_runtime_guard` 等)是测试基础设施,不属于稳定公开契约。
