use axum::{
    extract::{Path, Query},
    response::{Html, IntoResponse},
};
use maud::{html, Markup};
use serde::Deserialize;

use chrono::{DateTime, Utc};

use crate::config::{self, UiLanguage};
use crate::core;
use crate::web_support::{default_switch_target, lang_code, parse_language, tr};

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

pub(crate) async fn modal_switch_form(Query(q): Query<SwitchFormQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let from = q.from.as_deref().unwrap_or("claude");
    let target = default_switch_target(from);
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
                                option value="claude" selected[from == "claude"] { "Claude" }
                                option value="codex" selected[from == "codex"] { "Codex" }
                                option value="opencode" selected[from == "opencode"] { "OpenCode" }
                            }
                        }
                        div.field {
                            label for="to" { (tr(lang, "目标", "To")) }
                            select id="to" name="to" required {
                                option value="codex" selected[target == "codex"] { "Codex" }
                                option value="claude" selected[target == "claude"] { "Claude" }
                                option value="opencode" selected[target == "opencode"] { "OpenCode" }
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
                                option value="claude" { "Claude" }
                                option value="codex" { "Codex" }
                                option value="opencode" { "OpenCode" }
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
                            p { (tr(lang, "刷新列表后应能看到目标会话；也可以复制恢复命令在终端中打开。", "After refreshing the list, the target session should appear; you can also copy the resume command and open it in your terminal.")) }
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
                        button.invert type="button" data-modal=(format!("/modal/delete/exec/{}/{}?lang={}", provider, session_id, lang_code(lang))) { (tr(lang, "删除", "Delete")) }
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

    match config::update_web_preferences(q.per_page, language, Some(show_opencode_subagents)) {
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
