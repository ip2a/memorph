use axum::{
    extract::{Path, Query, RawQuery},
    response::{Html, IntoResponse},
};
use maud::{html, Markup, PreEscaped};
use serde::{Deserialize, Deserializer};

use chrono::{DateTime, Utc};

use crate::config::{self, UiLanguage};
use crate::web_support::{lang_code, parse_language, query_escape, tr};
use crate::{core, providers, shared};

#[derive(Deserialize)]
pub(crate) struct SwitchFormQuery {
    lang: Option<String>,
    from: Option<String>,
    session_id: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WorkspaceQuery {
    lang: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct LangQuery {
    lang: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SettingsExecQuery {
    lang: Option<String>,
    per_page: Option<usize>,
    show_opencode_subagents: Option<String>,
    auto_refresh_after_delete: Option<String>,
    agent_order: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    primary_agent: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct ExportExecQuery {
    lang: Option<String>,
    output_prefix: String,
    format: String,
}

#[derive(Deserialize)]
pub(crate) struct ImportExecQuery {
    lang: Option<String>,
    provider: String,
    file_or_id: String,
    workspace: String,
}

#[derive(Default)]
struct ShareCreateExecQuery {
    lang: Option<String>,
    workspace: Option<String>,
    title: Option<String>,
    targets: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct ShareBindExecQuery {
    lang: Option<String>,
    provider: String,
    session_id: Option<String>,
    workspace: String,
}

#[derive(Deserialize)]
pub(crate) struct ShareRemoveExecQuery {
    lang: Option<String>,
    delete_provider_sessions: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ShareRenameExecQuery {
    lang: Option<String>,
    title: String,
}

#[derive(Deserialize)]
pub(crate) struct SharePushExecQuery {
    lang: Option<String>,
}

pub(crate) async fn modal_share_form(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<WorkspaceQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let source = match core::get_session(&provider, &session_id) {
        Ok(session) => session,
        Err(e) => return Html(modal_error(e, lang).into_string()),
    };
    let workspaces = config::known_workspaces().unwrap_or_default();
    let workspace = q
        .workspace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(source.session.project_dir.as_deref())
        .unwrap_or("");
    let title = source.session.title.as_deref().unwrap_or("Shared session");
    let choices = share_target_providers(Some(&provider));
    let default_target = providers::default_switch_target(&provider);

    Html(
        html! {
            dialog.settings-modal {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "转为共享会话", "Create Shared Session")) }
                            p.modal-subtitle { (tr(lang, "源会话会成为第一个绑定；勾选的目标 AI 会写入同一份共享会话。", "The source session becomes the first binding; selected target AIs receive the same shared session.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action=(format!("/modal/share/exec/{}/{}", provider, query_escape(&session_id))) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.result-grid {
                            span { (tr(lang, "来源", "Source")) }
                            code { (provider_label(&provider)) " / " (session_id) }
                        }
                        div.field {
                            label for="share-title" { (tr(lang, "共享标题", "Shared Title")) }
                            input id="share-title" type="text" name="title" value=(title) required;
                        }
                        div.field {
                            label for="share-workspace" { (tr(lang, "共享目录", "Shared Directory")) }
                            input id="share-workspace" type="text" name="workspace" list="known-workspaces" value=(workspace) placeholder="/absolute/path" required;
                        }
                        div.field {
                            label { (tr(lang, "目标 AI", "Target AIs")) }
                            @if choices.is_empty() {
                                p.modal-subtitle { (tr(lang, "没有可写入的目标 AI。", "No writable target AI is available.")) }
                            } @else {
                                div.agent-picker role="group" aria-label=(tr(lang, "目标 AI", "Target AIs")) {
                                    @for choice in &choices {
                                        @let checked = choice.id == default_target;
                                        label.agent-pill {
                                            input type="checkbox" name="target" value=(choice.id) checked[checked];
                                            span { (choice.name) }
                                        }
                                    }
                                }
                            }
                        }
                        (workspace_datalist(&workspaces))
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "创建共享", "Create Shared")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_exec(
    Path((provider, session_id)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
    let q = ShareCreateExecQuery::from_raw(raw_query.as_deref());
    let lang = query_language(q.lang.as_deref());
    let workspace = q.workspace.unwrap_or_default();
    let workspace = workspace.trim();
    if workspace.is_empty() {
        return Html(
            modal_error(
                tr(lang, "共享目录不能为空", "Shared directory cannot be empty"),
                lang,
            )
            .into_string(),
        );
    }
    let targets = normalize_targets(&provider, q.targets);
    if targets.is_empty() {
        return Html(
            modal_error(
                tr(lang, "至少选择一个目标 AI", "Choose at least one target AI"),
                lang,
            )
            .into_string(),
        );
    }

    let params = shared::ShareCreateParams {
        provider,
        session_id,
        targets,
        to_dir: Some(workspace.to_string()),
        title: q.title.filter(|value| !value.trim().is_empty()),
    };

    match shared::create_group(&params) {
        Ok(result) => Html(render_share_created(result, lang).into_string()),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_share_bind_form(
    Path(group_id): Path<String>,
    Query(q): Query<WorkspaceQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let group = match shared::load_group(&group_id) {
        Ok(group) => group,
        Err(e) => return Html(modal_error(e, lang).into_string()),
    };
    let title = group.title;
    let workspaces = config::known_workspaces().unwrap_or_default();
    let workspace = q.workspace.as_deref().unwrap_or("");
    let choices = share_target_providers(None);

    Html(
        html! {
            dialog.settings-modal {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "新增订阅", "Add Holding")) }
                            p.modal-subtitle { (title) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action=(format!("/modal/share/bind/exec/{}", query_escape(&group_id))) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="share-bind-provider" { (tr(lang, "目标 AI", "Target AI")) }
                            select id="share-bind-provider" name="provider" required {
                                @for choice in &choices {
                                    option value=(choice.id) { (choice.name) }
                                }
                            }
                        }
                        div.field {
                            label for="share-bind-session" { (tr(lang, "已有 Session ID", "Existing Session ID")) }
                            input id="share-bind-session" type="text" name="session_id" placeholder=(tr(lang, "留空时创建新投影", "Leave empty to create a new projection"));
                            p.modal-subtitle { (tr(lang, "填入已有会话会把它纳入共享；留空会把当前共享内容写入目标 AI。", "Provide an existing session to join it, or leave empty to write the current shared content into the target AI.")) }
                        }
                        div.field {
                            label for="share-bind-workspace" { (tr(lang, "目标目录", "Target Directory")) }
                            input id="share-bind-workspace" type="text" name="workspace" list="known-workspaces" value=(workspace) placeholder="/absolute/path" required;
                        }
                        (workspace_datalist(&workspaces))
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "新增订阅", "Add Holding")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_bind_exec(
    Path(group_id): Path<String>,
    Query(q): Query<ShareBindExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let workspace = q.workspace.trim();
    if workspace.is_empty() {
        return Html(
            modal_error(
                tr(lang, "目标目录不能为空", "Target directory cannot be empty"),
                lang,
            )
            .into_string(),
        );
    }

    let params = shared::AddHoldingParams {
        group_id,
        provider: q.provider,
        session_id: q.session_id.filter(|value| !value.trim().is_empty()),
        to_dir: Some(workspace.to_string()),
    };

    match shared::add_holding(&params) {
        Ok(holding) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "订阅已新增", "Holding Added")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "订阅已生效", "Holding is active")) }
                            p { (tr(lang, "你可以随时从该 holding 推送同步到其他订阅方。", "You can push sync from this holding to other subscribers at any time.")) }
                        }
                        div.result-grid {
                            span { (tr(lang, "目标", "Target")) }
                            code { (provider_label(&holding.provider)) " / " (holding.session_id) }
                            span { (tr(lang, "工作区", "Workspace")) }
                            code { (holding.target_dir.as_deref().unwrap_or("-")) }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "稍后刷新", "Later")) }
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新状态", "Refresh Status")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_share_sync_all(Query(q): Query<LangQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    Html(
        modal_error(
            tr(
                lang,
                "请从共享详情页触发同步",
                "Please trigger sync from the shared detail page",
            ),
            lang,
        )
        .into_string(),
    )
}

pub(crate) async fn modal_share_sync(
    Path(group_id): Path<String>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    render_share_sync(group_id, lang)
}

pub(crate) async fn modal_share_push(
    Path((group_id, holding_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "推送同步", "Push Sync")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p { (tr(lang, "将此 holding 的当前内容覆盖到其他所有订阅方。", "Overwrite all other subscribers with this holding's current content.")) }
                    footer {
                        button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                        button.invert type="button" data-modal=(format!(
                            "/modal/share/push/exec/{}/{}?lang={}",
                            query_escape(&group_id),
                            query_escape(&holding_id),
                            lang_code(lang)
                        )) { (tr(lang, "推送同步", "Push Sync")) }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_push_exec(
    Path((group_id, holding_id)): Path<(String, String)>,
    Query(q): Query<SharePushExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match shared::push_sync(&group_id, &holding_id) {
        Ok(report) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "推送同步完成", "Push Sync Complete")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "同步已执行", "Sync executed")) }
                            p { (tr(lang, "内容已覆盖到目标订阅方。", "Content has been overwritten to target subscribers.")) }
                        }
                        div.result-grid {
                            span { (tr(lang, "来源", "Source")) }
                            code { (report.source_provider) }
                            span { (tr(lang, "成功", "Success")) }
                            code { (report.success.len()) }
                            span { (tr(lang, "错误", "Errors")) }
                            code { (report.errors.len()) }
                        }
                        @if !report.errors.is_empty() {
                            div.verify-block {
                                span.block-label { (tr(lang, "错误", "Errors")) }
                                pre {
                                    code {
                                        @for error in &report.errors {
                                            (error) "\n"
                                        }
                                    }
                                }
                            }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "稍后刷新", "Later")) }
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新状态", "Refresh Status")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_share_unbind(
    Path((group_id, holding_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "移除订阅", "Remove Holding")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p { (tr(lang, "移除后这个会话不会再参与共享同步。", "After removing, this session will no longer participate in shared sync.")) }
                    p { code { (holding_id) } }
                    footer {
                        button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                        button.invert type="button" data-modal=(format!(
                            "/modal/share/unbind/exec/{}/{}?lang={}",
                            query_escape(&group_id),
                            query_escape(&holding_id),
                            lang_code(lang)
                        )) { (tr(lang, "移除", "Remove")) }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_unbind_exec(
    Path((group_id, holding_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match shared::remove_holding(&group_id, &holding_id) {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "已移除", "Removed")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "订阅已移除。", "Holding has been removed.")) }
                        footer {
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新状态", "Refresh Status")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_share_remove(
    Path(group_id): Path<String>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "移除共享会话", "Remove Shared Session")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p { (tr(lang, "这会删除 memorph 的共享组记录。默认不会删除各 AI 中已经存在的会话。", "This removes memorph's shared group record. Existing AI sessions are not deleted by default.")) }
                    p { code { (group_id) } }
                    form method="get" action=(format!("/modal/share/remove/exec/{}", query_escape(&group_id))) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        label.settings-check {
                            input type="checkbox" name="delete_provider_sessions" value="true";
                            span { (tr(lang, "同时删除提供方会话", "Also delete provider sessions")) }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "移除", "Remove")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_remove_exec(
    Path(group_id): Path<String>,
    Query(q): Query<ShareRemoveExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match shared::delete_group(&group_id, q.delete_provider_sessions.is_some()) {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "已移除", "Removed")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "共享组记录已移除。", "Shared group record has been removed.")) }
                        footer {
                            button.invert type="button" onclick=(format!("closeModal(); goUrl('/shared?lang={}')", lang_code(lang))) { (tr(lang, "返回共享列表", "Back to Shared")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_share_rename_form(
    Path(group_id): Path<String>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let title = shared::load_group(&group_id)
        .map(|g| g.title)
        .unwrap_or_default();

    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "重命名共享会话", "Rename Shared Session")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action=(format!("/modal/share/rename/exec/{}", query_escape(&group_id))) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="shared-title" { (tr(lang, "新标题", "New Title")) }
                            input id="shared-title" type="text" name="title" value=(title) required;
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "保存", "Save")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_share_rename_exec(
    Path(group_id): Path<String>,
    Query(q): Query<ShareRenameExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match shared::rename_group(&group_id, &q.title) {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "已重命名", "Renamed")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { "title=" code { (q.title) } }
                        footer {
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新状态", "Refresh Status")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_switch_form(Query(q): Query<SwitchFormQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let from = q.from.as_deref().unwrap_or("claude");
    let target = providers::default_switch_target(from);
    let workspaces = config::known_workspaces().unwrap_or_default();
    let workspace = q.workspace.as_deref().unwrap_or("");
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "切换会话", "Switch Session")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action="/modal/switch/exec" data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="from" { (tr(lang, "来源", "From")) }
                            select id="from" name="from" required {
                                @for id in providers::all_provider_ids() {
                                    @let name = providers::find_provider(id).map(|p| p.name()).unwrap_or(id);
                                    option value=(id) selected[from == *id] { (name) }
                                }
                            }
                        }
                        div.field {
                            label for="to" { (tr(lang, "目标", "To")) }
                            select id="to" name="to" required {
                                @for id in providers::all_provider_ids() {
                                    @let name = providers::find_provider(id).map(|p| p.name()).unwrap_or(id);
                                    option value=(id) selected[target == *id] { (name) }
                                }
                            }
                        }
                        div.field {
                            label for="session_id" { "Session ID" }
                            input id="session_id" type="text" name="session_id" value=(q.session_id.as_deref().unwrap_or("")) placeholder=(tr(lang, "留空时使用当前目录最近会话", "Leave empty to use the latest session in current directory"));
                        }
                        div.field {
                            label for="switch-workspace" { (tr(lang, "目标工作区", "Target Workspace")) }
                            input id="switch-workspace" type="text" name="workspace" list="known-workspaces" value=(workspace) placeholder="/absolute/path" required;
                            p.modal-subtitle { (tr(lang, "可以填当前会话工作区，也可以改成另一个历史工作区。", "Use the current session workspace or change it to another known workspace.")) }
                        }
                        (workspace_datalist(&workspaces))
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "执行", "Run")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_export_form(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<WorkspaceQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let default_prefix = q
        .workspace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|workspace| format!("{}/{}", workspace.trim_end_matches(['/', '\\']), session_id))
        .unwrap_or_else(|| session_id.clone());

    Html(
        html! {
            dialog {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "导出会话", "Export Session")) }
                            p.modal-subtitle { (tr(lang, "支持导出 JSON、Markdown 和 HTML。", "Exports JSON, Markdown, and HTML.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action=(format!("/modal/export/exec/{}/{}", provider, session_id)) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="export-format" { (tr(lang, "格式", "Format")) }
                            select id="export-format" name="format" required {
                                option value="json" selected { "JSON" }
                                option value="md" { "Markdown" }
                                option value="html" { "HTML" }
                            }
                        }
                        div.field {
                            label for="output_prefix" { (tr(lang, "输出文件前缀", "Output Prefix")) }
                            input id="output_prefix" type="text" name="output_prefix" value=(default_prefix) required;
                            p.modal-subtitle { (tr(lang, "会写入服务端本地路径，并按格式自动追加后缀。", "Writes to a server-local path and appends the selected extension automatically.")) }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "导出", "Export")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_export_exec(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<ExportExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let output_prefix = q.output_prefix.trim();
    if output_prefix.is_empty() {
        return Html(
            modal_error(
                tr(
                    lang,
                    "输出文件前缀不能为空",
                    "Output prefix cannot be empty",
                ),
                lang,
            )
            .into_string(),
        );
    }
    let format = q.format.trim();
    if !matches!(format, "json" | "md" | "markdown" | "html") {
        return Html(
            modal_error(
                tr(
                    lang,
                    "当前 Web 导出只支持 json、md 和 html",
                    "Web export supports json, md, and html only",
                ),
                lang,
            )
            .into_string(),
        );
    }

    let params = core::ExportParams {
        provider,
        session_id,
        output_prefix: Some(output_prefix.to_string()),
        format: format.to_string(),
    };

    match core::export_session(&params) {
        Ok(result) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "导出完成", "Export Complete")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "会话已导出", "Session exported")) }
                            p { (tr(lang, "文件已经写入服务端本地路径。", "The file has been written to the server-local path.")) }
                        }
                        div.result-grid {
                            @for file in result.files {
                                span { (tr(lang, "文件", "File")) }
                                code { (file) }
                            }
                        }
                        footer {
                            button.invert type="button" onclick="closeModal()" { (tr(lang, "完成", "Done")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_import_form(Query(q): Query<WorkspaceQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let workspaces = config::known_workspaces().unwrap_or_default();
    let workspace = q.workspace.as_deref().unwrap_or("");

    Html(
        html! {
            dialog.settings-modal {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "导入到工作区", "Import Into Workspace")) }
                            p.modal-subtitle { (tr(lang, "支持服务端本地 .json、.md 和 .html 文件路径；浏览器文件上传属于下一阶段。", "Supports server-local .json, .md, and .html paths; browser upload is a next-phase item.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action="/modal/import/exec" data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="import-provider" { (tr(lang, "目标终端智能体", "Target Terminal Agent")) }
                            select id="import-provider" name="provider" required {
                                @for id in providers::all_provider_ids() {
                                    @let name = providers::find_provider(id).map(|p| p.name()).unwrap_or(id);
                                    option value=(id) { (name) }
                                }
                            }
                        }
                        div.field {
                            label for="file_or_id" { (tr(lang, "导入文件路径", "Import File Path")) }
                            input id="file_or_id" type="text" name="file_or_id" placeholder="/absolute/path/session.json|session.md|session.html" required;
                        }
                        div.field {
                            label for="import-workspace" { (tr(lang, "目标工作区", "Target Workspace")) }
                            input id="import-workspace" type="text" name="workspace" list="known-workspaces" value=(workspace) placeholder="/absolute/path" required;
                        }
                        (workspace_datalist(&workspaces))
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "导入", "Import")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_import_exec(Query(q): Query<ImportExecQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let file_or_id = q.file_or_id.trim();
    let workspace = q.workspace.trim();
    if file_or_id.is_empty() || workspace.is_empty() {
        return Html(
            modal_error(
                tr(
                    lang,
                    "文件路径和目标工作区不能为空",
                    "File path and target workspace cannot be empty",
                ),
                lang,
            )
            .into_string(),
        );
    }
    if !(file_or_id.ends_with(".json")
        || file_or_id.ends_with(".md")
        || file_or_id.ends_with(".html"))
    {
        return Html(
            modal_error(
                tr(
                    lang,
                    "当前 Web 导入只支持 .json、.md 和 .html 文件",
                    "Web import currently supports .json, .md, and .html files only",
                ),
                lang,
            )
            .into_string(),
        );
    }

    let params = core::ImportParams {
        provider: q.provider,
        file_or_id: file_or_id.to_string(),
        to_dir: Some(workspace.to_string()),
    };

    match core::import_session(&params) {
        Ok(result) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "导入完成", "Import Complete")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "导入成功", "Import succeeded")) }
                            p { (tr(lang, "目标终端智能体已经写入新的会话。", "A new session has been written to the target terminal agent.")) }
                        }
                        div.result-grid {
                            span { (tr(lang, "目标", "Target")) }
                            code { (result.provider_name) " / " (result.new_session_id) }
                            span { (tr(lang, "工作区", "Workspace")) }
                            code { (workspace) }
                        }
                        @if let Some(command) = result.resume_command {
                            div.verify-block {
                                span.block-label { (tr(lang, "恢复命令", "Resume Command")) }
                                pre { code { (command) } }
                            }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "稍后刷新", "Later")) }
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新列表", "Refresh List")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

#[derive(Deserialize)]
pub(crate) struct SwitchExecQuery {
    from: String,
    to: String,
    session_id: Option<String>,
    workspace: Option<String>,
    lang: Option<String>,
}

pub(crate) async fn modal_switch_exec(Query(q): Query<SwitchExecQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    if q.from == q.to {
        return Html(
            modal_error(
                tr(
                    lang,
                    "来源和目标不能相同",
                    "Source and target cannot be the same",
                ),
                lang,
            )
            .into_string(),
        );
    }

    let params = core::SwitchParams {
        from: q.from,
        to: q.to,
        session_id: q.session_id.filter(|value| !value.trim().is_empty()),
        to_dir: q.workspace.filter(|value| !value.trim().is_empty()),
    };

    match core::switch_session(&params) {
        Ok(result) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "切换完成", "Switch Complete")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "切换成功", "Switch succeeded")) }
                            p { (tr(lang, "目标终端智能体已经写入新的会话。", "A new session has been written to the target terminal agent.")) }
                        }
                        div.result-grid {
                            span { (tr(lang, "来源", "Source")) }
                            code { (result.from_name) " / " (result.source_session_id) }
                            span { (tr(lang, "目标", "Target")) }
                            code { (result.to_name) " / " (result.target_session_id) }
                        }
                        @if let Some(command) = result.resume_command {
                            div.verify-block {
                                span.block-label { (tr(lang, "恢复命令", "Resume Command")) }
                                pre { code { (command) } }
                            }
                        }
                        div.verify-block {
                            span.block-label { (tr(lang, "怎么验证", "How to Verify")) }
                            p {
                                @if result.to_name == "Cursor" {
                                    (tr(lang, "Cursor 需要重启才能看到新会话；memorph 列表刷新后即可看到。", "Cursor must be restarted to see the new session; it will appear immediately in the memorph list after refreshing."))
                                } @else {
                                    (tr(lang, "刷新列表后应能看到目标会话；也可以复制恢复命令在终端中打开。", "After refreshing the list, the target session should appear; you can also copy the resume command and open it in your terminal."))
                                }
                            }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "稍后刷新", "Later")) }
                            button.invert type="button" onclick="closeModal(); afterDeleteRefresh();" { (tr(lang, "刷新列表", "Refresh List")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_delete(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let auto_refresh = match config::web_preferences() {
        Ok(prefs) => prefs.auto_refresh_after_delete,
        Err(_) => false,
    };
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "删除会话", "Delete Session")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p { (tr(lang, "确认删除这个会话？", "Delete this session?")) }
                    p { code { (session_id) } }
                    footer {
                        button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                        @if auto_refresh {
                            button.invert type="button" data-delete-action=(format!("/modal/delete/exec/{}/{}?lang={}", provider, session_id, lang_code(lang))) { (tr(lang, "删除", "Delete")) }
                        } @else {
                            button.invert type="button" data-modal=(format!("/modal/delete/exec/{}/{}?lang={}", provider, session_id, lang_code(lang))) { (tr(lang, "删除", "Delete")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_delete_exec(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match core::delete_session(&provider, &session_id) {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "已删除", "Deleted")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "会话已删除。", "Session deleted.")) }
                        p.modal-subtitle { (tr(lang, "删除已经完成；返回列表只是在重新扫描并刷新会话列表。", "Deletion is complete; returning only rescans and refreshes the session list.")) }
                        footer {
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新列表", "Refresh List")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_rename_form(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "重命名会话", "Rename Session")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action=(format!("/modal/rename/exec/{}/{}", provider, session_id)) data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        div.field {
                            label for="title" { (tr(lang, "新标题", "New Title")) }
                            input id="title" type="text" name="title" required;
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "保存", "Save")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

#[derive(Deserialize)]
pub(crate) struct RenameExecQuery {
    title: String,
    lang: Option<String>,
}

pub(crate) async fn modal_rename_exec(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<RenameExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    match core::rename_session(&provider, &session_id, &q.title) {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "已重命名", "Renamed")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { "title=" code { (q.title) } }
                        footer {
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "完成", "Done")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

pub(crate) async fn modal_workspace_history(Query(q): Query<LangQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let workspaces = match config::known_workspaces() {
        Ok(workspaces) => workspaces,
        Err(e) => return Html(modal_error(e, lang).into_string()),
    };

    Html(
        html! {
            dialog.workspace-history-modal {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "历史工作空间", "Workspace History")) }
                            p.modal-subtitle { (tr(lang, "选择后会立即切换并记录到配置。", "Selecting one switches immediately and records it in the config.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    @if workspaces.is_empty() {
                        p { (tr(lang, "还没有记录工作空间。", "No workspaces have been recorded yet.")) }
                    } @else {
                        div.workspace-history-list {
                            @for workspace in workspaces {
                                button.workspace-history-item type="button" onclick=(format!("goWorkspace('{}')", js_string(&workspace.path))) {
                                    span.workspace-history-name { (workspace_name(&workspace.path)) }
                                    code { (workspace.path) }
                                    span.workspace-history-time { (format_workspace_time(workspace.last_viewed_at)) }
                                }
                            }
                        }
                    }
                    footer {
                        button.invert type="button" onclick="closeModal()" { (tr(lang, "完成", "Done")) }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_settings_form(Query(q): Query<WorkspaceQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let prefs = match config::web_preferences() {
        Ok(prefs) => prefs,
        Err(e) => return Html(modal_error(e, lang).into_string()),
    };
    let ordered_provider_ids = config::ordered_provider_ids(&prefs);
    let primary_provider_ids = config::primary_provider_ids(&prefs);
    let agent_order = ordered_provider_ids.join(",");

    Html(
        html! {
            dialog.settings-modal {
                article {
                    header {
                        div {
                            h3 { (tr(lang, "设置", "Settings")) }
                            p.modal-subtitle { (tr(lang, "设置会保存到 ~/.memorph/config.json。", "Settings are saved to ~/.memorph/config.json.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action="/modal/settings/exec" data-modal-form {
                        div.settings-list {
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "界面语言", "Interface Language")) }
                                    span { (tr(lang, "切换 Web 界面语言。", "Switch the Web interface language.")) }
                                }
                                select id="settings-lang" name="lang" required {
                                    option value="zh" selected[prefs.language == UiLanguage::Zh] { "中文" }
                                    option value="en" selected[prefs.language == UiLanguage::En] { "English" }
                                }
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "每智能体显示", "Per Agent")) }
                                    span { (tr(lang, "设置每个终端智能体默认显示的会话数量。", "Set how many sessions each terminal agent shows by default.")) }
                                }
                                input id="settings-per-page" name="per_page" type="number" min="1" max="200" value=(prefs.sessions_per_provider);
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { "OpenCode subagents" }
                                    span { (tr(lang, "显示 OpenCode 子 agent 会话。", "Show OpenCode subagent sessions.")) }
                                }
                                label.settings-check {
                                    input type="checkbox" name="show_opencode_subagents" value="true" checked[prefs.show_opencode_subagents];
                                    span { (tr(lang, "勾选", "Enabled")) }
                                }
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "删除后自动刷新", "Auto Refresh After Delete")) }
                                    span { (tr(lang, "删除会话后自动刷新列表，无需手动点击。", "Automatically refresh the session list after deletion.")) }
                                }
                                label.settings-check {
                                    input type="checkbox" name="auto_refresh_after_delete" value="true" checked[prefs.auto_refresh_after_delete];
                                    span { (tr(lang, "勾选", "Enabled")) }
                                }
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "Agent 显示顺序", "Agent Display Order")) }
                                    span { (tr(lang, "用逗号分隔 provider id；未列出的 agent 会自动排在后面。", "Comma-separated provider ids; omitted agents are appended automatically.")) }
                                }
                                input id="settings-agent-order" name="agent_order" type="text" value=(agent_order);
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "常用 Agent", "Primary Agents")) }
                                    span { (tr(lang, "勾选的 agent 直接显示；其他 agent 折叠到“更多”。全部勾选等同于不折叠。", "Checked agents are shown directly; the rest are folded under More. Checking all means no folding.")) }
                                }
                                div.settings-agent-list {
                                    @for id in &ordered_provider_ids {
                                        @let name = providers::find_provider(id).map(|p| p.name().to_string()).unwrap_or_else(|| id.clone());
                                        label.settings-check {
                                            input type="checkbox" name="primary_agent" value=(id) checked[primary_provider_ids.iter().any(|selected| selected == id)];
                                            span { (name) }
                                        }
                                    }
                                }
                            }
                            div.settings-row {
                                div.settings-copy {
                                    strong { (tr(lang, "版本", "Version")) }
                                    span { (format!("v{}", env!("CARGO_PKG_VERSION"))) }
                                }
                                a.button href="https://www.npmjs.com/package/memorph" target="_blank" rel="noopener noreferrer" { (tr(lang, "检查更新", "Check Update")) }
                            }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "保存", "Save")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

pub(crate) async fn modal_settings_exec(Query(q): Query<SettingsExecQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let language = q.lang.as_deref().and_then(parse_language);
    let show_opencode_subagents = q.show_opencode_subagents.is_some();
    let auto_refresh_after_delete = q.auto_refresh_after_delete.is_some();
    let agent_order = parse_provider_list(q.agent_order.as_deref().unwrap_or(""));

    let result = config::update_web_preferences(
        q.per_page,
        language,
        Some(show_opencode_subagents),
        Some(auto_refresh_after_delete),
    )
    .and_then(|_| config::update_agent_display_preferences(agent_order, q.primary_agent));

    match result {
        Ok(()) => Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "设置已保存", "Settings Saved")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "设置已经写入配置文件。", "Settings have been written to the config file.")) }
                        footer {
                            button.invert type="button" onclick=(format!("closeModal(); goUrl('/?lang={}')", lang_code(lang))) { (tr(lang, "刷新页面", "Refresh Page")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

fn parse_provider_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

struct ProviderChoice {
    id: String,
    name: String,
}

impl ShareCreateExecQuery {
    fn from_raw(raw_query: Option<&str>) -> Self {
        let mut query = Self::default();
        let Some(raw_query) = raw_query else {
            return query;
        };

        for (key, value) in parse_query_pairs(raw_query) {
            match key.as_str() {
                "lang" => query.lang = Some(value),
                "workspace" => query.workspace = Some(value),
                "title" => query.title = Some(value),
                "target" | "targets" | "target[]" | "targets[]" => query.targets.push(value),
                _ => {}
            }
        }

        query
    }
}

fn render_share_created(result: shared::SharedGroup, lang: UiLanguage) -> Markup {
    let detail_href = format!(
        "/shared/{}?lang={}",
        query_escape(&result.id),
        lang_code(lang)
    );

    html! {
        dialog.switch-result-modal {
            article {
                header {
                    h3 { (tr(lang, "共享会话已创建", "Shared Session Created")) }
                    button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                }
                div.success-callout {
                    strong { (tr(lang, "共享模式已启用", "Shared mode is active")) }
                    p { (tr(lang, "手动推送同步可将某一端的内容覆盖到其他订阅方。", "Manually push sync to overwrite other subscribers with one side's content.")) }
                }
                div.result-grid {
                    span { (tr(lang, "共享 ID", "Shared ID")) }
                    code { (result.id) }
                    span { (tr(lang, "订阅数量", "Holdings")) }
                    code { (result.holdings.len()) }
                }
                div.verify-block {
                    span.block-label { (tr(lang, "订阅", "Holdings")) }
                    div.binding-strip {
                        @for holding in &result.holdings {
                            span.status-pill {
                                (provider_label(&holding.provider)) ":" (short_id(&holding.session_id))
                            }
                        }
                    }
                }
                footer {
                    button type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新列表", "Refresh List")) }
                    a.button.invert href=(detail_href) { (tr(lang, "打开共享详情", "Open Detail")) }
                }
            }
        }
    }
}

fn render_share_sync(group_id: String, lang: UiLanguage) -> Html<String> {
    match shared::sync_to_latest(&group_id) {
        Ok(report) => Html(
            html! {
                dialog.switch-result-modal {
                    article {
                        header {
                            h3 { (tr(lang, "同步完成", "Sync Complete")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        div.success-callout {
                            strong { (tr(lang, "共享同步已执行", "Shared sync has run")) }
                            p { (tr(lang, "最新版本已覆盖到所有订阅方。", "The latest version has been overwritten to all subscribers.")) }
                        }
                        div.result-grid {
                            span { (tr(lang, "来源", "Source")) }
                            code { (report.source_provider) }
                            span { (tr(lang, "成功", "Success")) }
                            code { (report.success.len()) }
                            span { (tr(lang, "错误", "Errors")) }
                            code { (report.errors.len()) }
                        }
                        @if !report.errors.is_empty() {
                            div.verify-block {
                                span.block-label { (tr(lang, "错误", "Errors")) }
                                pre {
                                    code {
                                        @for error in &report.errors {
                                            (error) "\n"
                                        }
                                    }
                                }
                            }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "稍后刷新", "Later")) }
                            button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新状态", "Refresh Status")) }
                        }
                    }
                }
            }
            .into_string(),
        ),
        Err(e) => Html(modal_error(e, lang).into_string()),
    }
}

fn share_target_providers(exclude: Option<&str>) -> Vec<ProviderChoice> {
    providers::all_provider_ids()
        .iter()
        .filter(|id| exclude != Some(**id))
        .filter_map(|id| {
            let provider = providers::find_provider(id)?;
            provider.capabilities().write.then(|| ProviderChoice {
                id: (*id).to_string(),
                name: provider.name().to_string(),
            })
        })
        .collect()
}

fn normalize_targets(source_provider: &str, targets: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for target in targets {
        let target = target.trim();
        if target.is_empty() || target == source_provider || normalized.iter().any(|v| v == target)
        {
            continue;
        }
        normalized.push(target.to_string());
    }
    normalized
}

fn parse_query_pairs(raw_query: &str) -> Vec<(String, String)> {
    raw_query
        .split('&')
        .filter(|value| !value.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (query_unescape(key), query_unescape(value))
        })
        .collect()
}

fn query_unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        index += 3;
                        continue;
                    }
                }
                out.push(bytes[index]);
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn provider_label(provider_id: &str) -> String {
    providers::find_provider(provider_id)
        .map(|provider| provider.name().to_string())
        .unwrap_or_else(|| provider_id.to_string())
}

fn short_id(value: &str) -> String {
    let count = value.chars().count();
    if count <= 12 {
        value.to_string()
    } else {
        let prefix: String = value.chars().take(8).collect();
        format!("{prefix}...")
    }
}

fn query_language(value: Option<&str>) -> UiLanguage {
    value.and_then(parse_language).unwrap_or_default()
}

fn modal_error(error: impl std::fmt::Display, lang: UiLanguage) -> Markup {
    html! {
        dialog {
            article {
                header {
                    h3 { (tr(lang, "错误", "Error")) }
                    button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                }
                p { (error) }
                footer {
                    button.invert type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                }
            }
        }
    }
}

fn workspace_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .next_back()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn format_workspace_time(timestamp: i64) -> String {
    let datetime = if timestamp.abs() >= 1_000_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(timestamp)
    } else {
        DateTime::<Utc>::from_timestamp(timestamp, 0)
    };
    datetime
        .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn workspace_datalist(workspaces: &[config::WorkspaceEntry]) -> Markup {
    html! {
        datalist id="known-workspaces" {
            @for workspace in workspaces {
                option value=(workspace.path) {}
            }
        }
    }
}

fn js_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

// ---------------------------------------------------------------------------
// Session Manager
// ---------------------------------------------------------------------------

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> serde::de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("string or list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Vec<String>, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value.to_owned()])
        }

        fn visit_seq<S>(self, visitor: S) -> Result<Vec<String>, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            Deserialize::deserialize(serde::de::value::SeqAccessDeserializer::new(visitor))
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

#[derive(Deserialize)]
pub(crate) struct ManagerFormQuery {
    lang: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ManagerPreviewQuery {
    lang: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    provider: Vec<String>,
    days: Option<u32>,
    size_mb: Option<u32>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ManagerExecQuery {
    lang: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    sel: Vec<String>,
    output_dir: Option<String>,
}

/// Render the manager form modal.
pub(crate) async fn modal_manager_form(Query(q): Query<ManagerFormQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let workspace = q.workspace.as_deref().unwrap_or("");
    let provider_ids = providers::all_provider_ids();

    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "会话管理器", "Session Manager")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form #manager-filter method="get" action="/modal/manager/preview" data-modal-form {
                        input type="hidden" name="lang" value=(lang_code(lang));
                        input type="hidden" name="workspace" value=(workspace);
                        div.field {
                            label { (tr(lang, "编辑器", "Editors")) }
                            div.provider-checkboxes {
                                @for pid in provider_ids {
                                    @let name = providers::find_provider(pid).map(|p| p.name()).unwrap_or(pid);
                                    label.provider-pill {
                                        input type="checkbox" name="provider" value=(pid) checked;
                                        span { (name) }
                                    }
                                }
                            }
                        }
                        div.field {
                            label { (tr(lang, "超过 N 天未更新", "Older than N days")) }
                            input type="number" name="days" value="30" min="0";
                            p.modal-subtitle { (tr(lang, "0 表示不限", "0 means no limit")) }
                        }
                        div.field {
                            label { (tr(lang, "超过 N MB", "Larger than N MB")) }
                            input type="number" name="size_mb" value="10" min="0";
                            p.modal-subtitle { (tr(lang, "0 表示不限", "0 means no limit")) }
                        }
                        footer {
                            button type="button" onclick="closeModal()" { (tr(lang, "取消", "Cancel")) }
                            button.invert type="submit" { (tr(lang, "预览", "Preview")) }
                        }
                    }
                }
            }
        }
        .into_string(),
    )
}

/// Render the manager preview results inside the modal.
pub(crate) async fn modal_manager_preview(
    Query(q): Query<ManagerPreviewQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());

    let filter = crate::core::manager::ManagerFilter {
        providers: q.provider.clone(),
        older_than_days: q.days,
        larger_than_mb: q.size_mb,
        workspace: q.workspace.clone(),
    };

    let result = match crate::core::manager::preview(&filter) {
        Ok(r) => r,
        Err(e) => {
            return Html(
                html! {
                    div #manager-results {
                        p.error { (format!("Preview error: {}", e)) }
                    }
                }
                .into_string(),
            );
        }
    };

    let total_mb = result.total_size_bytes as f64 / (1024.0 * 1024.0);

    Html(
        html! {
            div #manager-results {
                p.manager-stats {
                    strong { (tr(lang, "符合条件", "Matching")) ": " }
                    span { (result.total_count) " " (tr(lang, "个会话", "sessions")) ", " }
                    span { (format!("{:.1} MB", total_mb)) }
                }

                @if result.items.is_empty() {
                    p { (tr(lang, "没有符合条件的会话。", "No sessions match the criteria.")) }
                } @else {
                    div.manager-table-wrapper {
                        table.manager-table {
                            thead {
                                tr {
                                    th {
                                        input type="checkbox" id="manager-select-all" checked onchange="toggleManagerSelectAll(this)";
                                    }
                                    th { (tr(lang, "编辑器", "Editor")) }
                                    th { (tr(lang, "标题", "Title")) }
                                    th { (tr(lang, "大小", "Size")) }
                                    th { (tr(lang, "上次更新", "Last Updated")) }
                                }
                            }
                            tbody {
                                @for item in &result.items {
                                    @let sel_val = format!("{}::{}", item.provider_id, item.session_id);
                                    @let size_mb = item.size_bytes as f64 / (1024.0 * 1024.0);
                                    @let time_str = item.last_active_at.map(format_workspace_time).unwrap_or_else(|| "-".to_string());
                                    tr {
                                        td {
                                            input type="checkbox" name="sel" value=(sel_val) checked class="manager-row-check";
                                        }
                                        td { (item.provider_name) }
                                        td {
                                            (item.title.as_deref().unwrap_or("(untitled)"))
                                            @if let Some(dir) = &item.project_dir {
                                                div.manager-meta-line {
                                                    span.meta-item { (dir) }
                                                }
                                            }
                                        }
                                        td { (format!("{:.1} MB", size_mb)) }
                                        td { (time_str) }
                                    }
                                }
                            }
                        }
                    }

                    div.manager-actions {
                        button.invert type="button" data-manager-action="/modal/manager/exec/clean" onclick="runManagerAction(this)" {
                            (tr(lang, "一键清理", "Clean Selected"))
                        }
                        button type="button" data-manager-action="/modal/manager/exec/backup" onclick="runManagerBackup(this)" {
                            (tr(lang, "一键备份", "Backup Selected"))
                        }
                    }
                }
            }

            script {
                (PreEscaped(r##"
                function toggleManagerSelectAll(cb) {
                    document.querySelectorAll('.manager-row-check').forEach(function(el) {
                        el.checked = cb.checked;
                    });
                }
                function runManagerAction(btn) {
                    var action = btn.dataset.managerAction;
                    var sels = Array.from(document.querySelectorAll('.manager-row-check:checked')).map(function(el) {
                        return 'sel=' + encodeURIComponent(el.value);
                    });
                    if (sels.length === 0) { alert('No sessions selected'); return; }
                    var url = action + '?' + sels.join('&');
                    loadModal(url);
                }
                function runManagerBackup(btn) {
                    var dir = prompt('Backup directory:');
                    if (!dir) return;
                    var action = btn.dataset.managerAction;
                    var sels = Array.from(document.querySelectorAll('.manager-row-check:checked')).map(function(el) {
                        return 'sel=' + encodeURIComponent(el.value);
                    });
                    if (sels.length === 0) { alert('No sessions selected'); return; }
                    var url = action + '?' + sels.join('&') + '&output_dir=' + encodeURIComponent(dir);
                    loadModal(url);
                }
                "##))
            }
        }
        .into_string(),
    )
}

/// Execute clean action.
pub(crate) async fn modal_manager_clean_exec(
    Query(q): Query<ManagerExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());

    let items = parse_manager_selections(&q.sel);
    if items.is_empty() {
        return Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "无选中项", "No Selection")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "未选择任何会话。", "No sessions were selected.")) }
                    }
                }
            }
            .into_string(),
        );
    }

    let result = crate::core::manager::clean(&items);
    let freed_mb = result.freed_bytes as f64 / (1024.0 * 1024.0);

    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "清理完成", "Clean Complete")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p {
                        strong { (result.success) } " " (tr(lang, "个已清理", "cleaned")) ", "
                        strong { (result.failed) } " " (tr(lang, "个失败", "failed"))
                    }
                    p { (format!("{:.1} MB {}", freed_mb, tr(lang, "已释放", "freed"))) }
                    @if !result.errors.is_empty() {
                        details {
                            summary { (tr(lang, "查看错误", "View Errors")) }
                            ul {
                                @for err in &result.errors {
                                    li { code { (err) } }
                                }
                            }
                        }
                    }
                    footer {
                        button.invert type="button" onclick="closeModal(); refreshMain();" { (tr(lang, "刷新列表", "Refresh List")) }
                    }
                }
            }
        }
        .into_string(),
    )
}

/// Execute backup action.
pub(crate) async fn modal_manager_backup_exec(
    Query(q): Query<ManagerExecQuery>,
) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());

    let items = parse_manager_selections(&q.sel);
    if items.is_empty() {
        return Html(
            html! {
                dialog {
                    article {
                        header {
                            h3 { (tr(lang, "无选中项", "No Selection")) }
                            button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                        }
                        p { (tr(lang, "未选择任何会话。", "No sessions were selected.")) }
                    }
                }
            }
            .into_string(),
        );
    }

    let output_dir = q.output_dir.as_deref().unwrap_or("./backups");
    let result = crate::core::manager::backup(&items, std::path::Path::new(output_dir));

    Html(
        html! {
            dialog {
                article {
                    header {
                        h3 { (tr(lang, "备份完成", "Backup Complete")) }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    p {
                        strong { (result.success) } " " (tr(lang, "个已备份", "backed up")) ", "
                        strong { (result.failed) } " " (tr(lang, "个失败", "failed"))
                    }
                    @if !result.files.is_empty() {
                        details {
                            summary { (tr(lang, "查看文件", "View Files")) }
                            ul {
                                @for f in &result.files {
                                    li { code { (f) } }
                                }
                            }
                        }
                    }
                    @if !result.errors.is_empty() {
                        details {
                            summary { (tr(lang, "查看错误", "View Errors")) }
                            ul {
                                @for err in &result.errors {
                                    li { code { (err) } }
                                }
                            }
                        }
                    }
                    footer {
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                }
            }
        }
        .into_string(),
    )
}

/// Parse `provider_id::session_id` selections into ManagerItems by re-scanning.
fn parse_manager_selections(sels: &[String]) -> Vec<crate::core::manager::ManagerItem> {
    let mut items = Vec::new();
    for sel in sels {
        let parts: Vec<&str> = sel.split("::").collect();
        if parts.len() != 2 {
            continue;
        }
        let provider_id = parts[0];
        let session_id = parts[1];

        let provider = match providers::find_provider(provider_id) {
            Some(p) => p,
            None => continue,
        };

        // Re-scan to find metadata and size
        let sessions = match provider.scan_sessions() {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Some(meta) = sessions.iter().find(|s| s.session_id == session_id) {
            let size_bytes = provider.session_size(session_id).unwrap_or(0);
            items.push(crate::core::manager::ManagerItem {
                provider_id: provider_id.to_string(),
                provider_name: provider.name().to_string(),
                session_id: session_id.to_string(),
                source_path: meta.source_path.clone(),
                title: meta.title.clone(),
                project_dir: meta.project_dir.clone(),
                last_active_at: meta.last_active_at,
                size_bytes,
            });
        }
    }
    items
}
