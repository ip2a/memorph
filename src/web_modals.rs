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
}

#[derive(Deserialize)]
pub(crate) struct LangQuery {
    lang: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SettingsExecQuery {
    lang: Option<String>,
    show_opencode_subagents: Option<String>,
}

pub(crate) async fn modal_switch_form(Query(q): Query<SwitchFormQuery>) -> impl IntoResponse {
    let lang = query_language(q.lang.as_deref());
    let from = q.from.as_deref().unwrap_or("claude");
    let target = default_switch_target(from);
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

#[derive(Deserialize)]
pub(crate) struct SwitchExecQuery {
    from: String,
    to: String,
    session_id: Option<String>,
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
        to_dir: None,
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

pub(crate) async fn modal_settings_form(Query(q): Query<LangQuery>) -> impl IntoResponse {
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
                            p.modal-subtitle { (tr(lang, "这些设置会保存到 ~/.memorph/config.json。", "These settings are saved to ~/.memorph/config.json.")) }
                        }
                        button type="button" onclick="closeModal()" { (tr(lang, "关闭", "Close")) }
                    }
                    form method="get" action="/modal/settings/exec" data-modal-form {
                        div.field {
                            label for="settings-lang" { (tr(lang, "界面语言", "Interface Language")) }
                            select id="settings-lang" name="lang" required {
                                option value="zh" selected[prefs.language == UiLanguage::Zh] { "中文" }
                                option value="en" selected[prefs.language == UiLanguage::En] { "English" }
                            }
                        }
                        div.settings-toggle-row {
                            label.agent-pill {
                                input type="checkbox" name="show_opencode_subagents" value="true" checked[prefs.show_opencode_subagents];
                                span { "OpenCode subagents" }
                            }
                            p.modal-subtitle id="opencode-subagents-help" {
                                (tr(lang, "显示标题中包含“(@... subagent)”的 OpenCode 子 agent 会话。", "Show OpenCode subagent sessions whose title contains “(@... subagent)”."))
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

    match config::update_web_preferences(None, language, Some(show_opencode_subagents)) {
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

fn js_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
