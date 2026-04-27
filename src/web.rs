use axum::{
    extract::{Path, RawQuery},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use std::path::{Path as FsPath, PathBuf};

use crate::config::{self, UiLanguage, WebPreferences};
use crate::core;
use crate::model::{ContentBlock, MemorphRole};
use crate::web_assets::{modal_script, MEMORPH_ASCII};
use crate::web_support::{
    home_url, html_lang, lang_code, parse_language, query_escape, simple_markdown, tr,
    truncate_chars, truncate_json, workspace_name,
};

pub fn router() -> Router {
    Router::new()
        .route("/", get(page_home))
        .route("/sessions/{provider}/{session_id}", get(page_session))
        .route("/modal/switch", get(crate::web_modals::modal_switch_form))
        .route(
            "/modal/switch/exec",
            get(crate::web_modals::modal_switch_exec),
        )
        .route(
            "/modal/delete/{provider}/{session_id}",
            get(crate::web_modals::modal_delete),
        )
        .route(
            "/modal/delete/exec/{provider}/{session_id}",
            get(crate::web_modals::modal_delete_exec),
        )
        .route(
            "/modal/rename/{provider}/{session_id}",
            get(crate::web_modals::modal_rename_form),
        )
        .route(
            "/modal/rename/exec/{provider}/{session_id}",
            get(crate::web_modals::modal_rename_exec),
        )
        .route(
            "/modal/workspaces",
            get(crate::web_modals::modal_workspace_history),
        )
        .route(
            "/modal/settings",
            get(crate::web_modals::modal_settings_form),
        )
        .route(
            "/modal/settings/exec",
            get(crate::web_modals::modal_settings_exec),
        )
        .route("/assets/style.css", get(crate::web_assets::serve_css))
        .route("/favicon.ico", get(crate::web_assets::serve_favicon))
}

fn layout(title: &str, content: Markup, lang: UiLanguage) -> Markup {
    html! {
        (DOCTYPE)
        html lang=(html_lang(lang)) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                link rel="stylesheet" href="/assets/style.css";
            }
            body {
                div.atomic-shell {
                    nav.topbar {
                        a.brand href=(format!("/?lang={}", lang_code(lang))) aria-label="memorph home" {
                            span.brand-mark { "m" }
                            span { "memorph" }
                        }
                        div.top-actions {
                            button.github-link type="button" data-modal=(format!("/modal/settings?lang={}", lang_code(lang))) aria-label=(tr(lang, "设置", "Settings")) title=(tr(lang, "设置", "Settings")) {
                                (PreEscaped(settings_icon_svg()))
                            }
                            a.github-link href="https://github.com/whillhill/memorph" target="_blank" rel="noopener noreferrer" aria-label="GitHub repository" title="GitHub" {
                                (PreEscaped(github_icon_svg()))
                            }
                            label.lang-switch {
                                span { (tr(lang, "语种", "Language")) }
                                select name="lang" aria-label=(tr(lang, "语种", "Language")) onchange="setLanguage(this.value)" {
                                    option value="zh" selected[lang == UiLanguage::Zh] { "中文" }
                                    option value="en" selected[lang == UiLanguage::En] { "English" }
                                }
                            }
                            span.version { (format!("v{}", env!("CARGO_PKG_VERSION"))) }
                        }
                    }
                    main {
                        (content)
                    }
                }
                div #modal-container {}
                div.loading-layer aria-live="polite" aria-label=(tr(lang, "正在加载", "Loading")) {
                    div.loading-card {
                        span.loading-spinner aria-hidden="true" {}
                        span { (tr(lang, "正在加载", "Loading")) }
                    }
                }
                (modal_script())
            }
        }
    }
}

fn github_icon_svg() -> &'static str {
    r#"<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.6 7.6 0 0 1 8 3.86c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>"#
}

fn settings_icon_svg() -> &'static str {
    r#"<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path fill="currentColor" d="M8 1.5c.43 0 .8.3.89.72l.16.76c.3.09.59.21.86.36l.65-.42a.92.92 0 0 1 1.14.12l.76.76c.3.3.35.77.12 1.14l-.42.65c.15.27.27.56.36.86l.76.16c.42.09.72.46.72.89v1.08c0 .43-.3.8-.72.89l-.76.16c-.09.3-.21.59-.36.86l.42.65c.23.37.18.84-.12 1.14l-.76.76a.92.92 0 0 1-1.14.12l-.65-.42c-.27.15-.56.27-.86.36l-.16.76c-.09.42-.46.72-.89.72H6.92c-.43 0-.8-.3-.89-.72l-.16-.76a4.9 4.9 0 0 1-.86-.36l-.65.42a.92.92 0 0 1-1.14-.12l-.76-.76a.92.92 0 0 1-.12-1.14l.42-.65a4.9 4.9 0 0 1-.36-.86l-.76-.16a.92.92 0 0 1-.72-.89V7.46c0-.43.3-.8.72-.89l.76-.16c.09-.3.21-.59.36-.86l-.42-.65a.92.92 0 0 1 .12-1.14l.76-.76c.3-.3.77-.35 1.14-.12l.65.42c.27-.15.56-.27.86-.36l.16-.76c.09-.42.46-.72.89-.72H8Zm-.5 1.2-.18.86-.42.12a3.5 3.5 0 0 0-1.05.44l-.37.22-.74-.48-.62.62.48.74-.22.37c-.2.32-.34.67-.44 1.05l-.12.42-.86.18v.88l.86.18.12.42c.1.38.24.73.44 1.05l.22.37-.48.74.62.62.74-.48.37.22c.32.2.67.34 1.05.44l.42.12.18.86h.88l.18-.86.42-.12c.38-.1.73-.24 1.05-.44l.37-.22.74.48.62-.62-.48-.74.22-.37c.2-.32.34-.67.44-1.05l.12-.42.86-.18v-.88l-.86-.18-.12-.42a3.5 3.5 0 0 0-.44-1.05l-.22-.37.48-.74-.62-.62-.74.48-.37-.22a3.5 3.5 0 0 0-1.05-.44l-.42-.12-.18-.86H7.5ZM8 5.3a2.7 2.7 0 1 1 0 5.4 2.7 2.7 0 0 1 0-5.4Zm0 1.2a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3Z"/></svg>"#
}

fn apply_web_preferences(query: &HomeQuery) -> anyhow::Result<WebPreferences> {
    let language = query.lang.as_deref().and_then(parse_language);
    if query.per_page.is_some() || language.is_some() {
        config::update_web_preferences(query.per_page, language, None)?;
    }
    config::web_preferences()
}

#[derive(Default)]
struct HomeQuery {
    workspace: Option<String>,
    provider: Vec<String>,
    q: Option<String>,
    sort: Option<String>,
    per_page: Option<usize>,
    visible: Option<usize>,
    lang: Option<String>,
}

async fn page_home(RawQuery(raw_query): RawQuery) -> impl IntoResponse {
    let q = HomeQuery::from_raw(raw_query.as_deref());
    let prefs = match apply_web_preferences(&q) {
        Ok(prefs) => prefs,
        Err(e) => {
            return Html(layout("memorph - 错误", error_markup(e), UiLanguage::Zh).into_string())
        }
    };

    let known_workspaces = match config::known_workspaces() {
        Ok(items) => items,
        Err(e) => {
            return Html(layout("memorph - 错误", error_markup(e), prefs.language).into_string())
        }
    };

    let saved_workspace = match config::selected_workspace() {
        Ok(value) => value,
        Err(e) => {
            return Html(layout("memorph - 错误", error_markup(e), prefs.language).into_string())
        }
    };

    let selected = q
        .workspace
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(saved_workspace.as_deref())
        .or_else(|| known_workspaces.first().map(|entry| entry.path.as_str()));

    let workspace = match config::resolve_workspace(selected) {
        Ok(path) => path,
        Err(e) => {
            return Html(layout("memorph - 错误", error_markup(e), prefs.language).into_string())
        }
    };

    if let Err(e) = config::remember_workspace(&workspace) {
        return Html(layout("memorph - 错误", error_markup(e), prefs.language).into_string());
    };

    let content = render_session_list(
        &workspace,
        &known_workspaces,
        &q.provider,
        q.q.as_deref(),
        q.sort.as_deref(),
        q.visible,
        &prefs,
    );
    Html(layout("memorph", content, prefs.language).into_string())
}

fn render_session_list(
    workspace: &PathBuf,
    _known_workspaces: &[config::WorkspaceEntry],
    provider_filter: &[String],
    search: Option<&str>,
    sort: Option<&str>,
    visible: Option<usize>,
    prefs: &WebPreferences,
) -> Markup {
    let workspace_str = workspace.to_string_lossy().to_string();
    let lang = prefs.language;
    let per_page = prefs.sessions_per_provider.clamp(1, 200);
    let visible_limit = visible.unwrap_or(per_page).clamp(1, 1000);
    let providers = normalize_provider_filter(provider_filter);

    let groups_result = core::list_sessions(&core::SessionListParams {
        all: false,
        providers: providers.clone(),
        cwd: Some(workspace_str.clone()),
    });

    let groups = match groups_result {
        Ok(groups) => sort_groups(
            filter_groups(filter_opencode_subagents(groups, prefs), search),
            sort,
        ),
        Err(e) => return error_markup(e),
    };
    let total: usize = groups.iter().map(|group| group.sessions.len()).sum();

    html! {
        section.ascii-banner aria-label="memorph" {
            pre { (MEMORPH_ASCII) }
        }

        section.workspace-panel {
            div.workspace-hero {
                p.eyebrow { (tr(lang, "工作空间", "Workspace")) }
                h1 { (workspace_name(&workspace_str)) }
                p.workspace-path { (workspace_str) }
            }

            form.filter-panel method="get" action="/" {
                div.filter-row.filter-row-main {
                    div.field.field-wide {
                        label for="workspace" { (tr(lang, "工作空间路径", "Workspace Path")) }
                        div.workspace-combo {
                            input id="workspace" name="workspace" type="text" value=(workspace_str) placeholder="/absolute/path";
                            button.invert type="submit" { (tr(lang, "前往", "Go")) }
                            button type="button" data-modal=(format!("/modal/workspaces?lang={}", lang_code(lang))) { (tr(lang, "历史", "History")) }
                        }
                    }
                    div.field.agent-field {
                        label for="provider" { (tr(lang, "终端智能体", "Terminal Agent")) }
                        div.agent-picker id="provider" role="group" aria-label=(tr(lang, "终端智能体", "Terminal Agent")) {
                            label.agent-pill {
                                input type="checkbox" name="provider" value="claude" checked[providers.iter().any(|provider| provider == "claude")];
                                span { "Claude" }
                            }
                            label.agent-pill {
                                input type="checkbox" name="provider" value="codex" checked[providers.iter().any(|provider| provider == "codex")];
                                span { "Codex" }
                            }
                            label.agent-pill {
                                input type="checkbox" name="provider" value="opencode" checked[providers.iter().any(|provider| provider == "opencode")];
                                span { "OpenCode" }
                            }
                        }
                    }
                }
                div.filter-row.filter-row-compact {
                    div.field {
                        label for="q" { (tr(lang, "搜索", "Search")) }
                        input id="q" name="q" type="search" placeholder=(tr(lang, "标题或 ID", "Title or ID")) value=(search.unwrap_or(""));
                    }
                    div.field.field-number {
                        label for="per_page" { (tr(lang, "每智能体显示", "Per Agent")) }
                        input id="per_page" name="per_page" type="number" min="1" max="200" value=(per_page);
                    }
                    div.field {
                        label for="sort" { (tr(lang, "排序", "Sort")) }
                        select id="sort" name="sort" onchange="submitWithLoading(this.form)" {
                            option value="updated_desc" selected[sort.unwrap_or("updated_desc") == "updated_desc"] { (tr(lang, "最新修改", "Newest")) }
                            option value="updated_asc" selected[sort == Some("updated_asc")] { (tr(lang, "最早修改", "Oldest")) }
                            option value="title_asc" selected[sort == Some("title_asc")] { (tr(lang, "标题 A-Z", "Title A-Z")) }
                            option value="title_desc" selected[sort == Some("title_desc")] { (tr(lang, "标题 Z-A", "Title Z-A")) }
                            option value="size_desc" selected[sort == Some("size_desc")] { (tr(lang, "内容大小", "Largest")) }
                        }
                    }
                    button.invert type="submit" { (tr(lang, "应用", "Apply")) }
                }
                input type="hidden" name="lang" value=(lang_code(lang));
            }
        }

        (render_session_groups(&groups, total, visible_limit, per_page, &workspace_str, &providers, search, sort, lang))
    }
}

fn normalize_provider_filter(provider_filter: &[String]) -> Vec<String> {
    let providers: Vec<String> = provider_filter
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();

    if providers.is_empty() {
        vec![
            "claude".to_string(),
            "codex".to_string(),
            "opencode".to_string(),
        ]
    } else {
        providers
    }
}

fn filter_opencode_subagents(
    groups: Vec<core::SessionGroup>,
    prefs: &WebPreferences,
) -> Vec<core::SessionGroup> {
    if prefs.show_opencode_subagents {
        return groups;
    }

    groups
        .into_iter()
        .filter_map(|mut group| {
            if group.provider_id == "opencode" {
                group
                    .sessions
                    .retain(|session| !is_opencode_subagent_title(session.title.as_deref()));
            }
            (!group.sessions.is_empty()).then_some(group)
        })
        .collect()
}

fn is_opencode_subagent_title(title: Option<&str>) -> bool {
    let Some(title) = title else {
        return false;
    };
    title.contains("(@") && title.contains(" subagent)")
}

impl HomeQuery {
    fn from_raw(raw_query: Option<&str>) -> Self {
        let mut query = Self::default();
        let Some(raw_query) = raw_query else {
            return query;
        };

        for pair in raw_query.split('&').filter(|value| !value.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = query_unescape(key);
            let value = query_unescape(value);
            match key.as_str() {
                "workspace" => query.workspace = Some(value),
                "provider" | "provider[]" => query.provider.push(value),
                "q" => query.q = Some(value),
                "sort" => query.sort = Some(value),
                "per_page" => query.per_page = value.parse().ok(),
                "visible" => query.visible = value.parse().ok(),
                "lang" => query.lang = Some(value),
                _ => {}
            }
        }

        query
    }
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

fn filter_groups(groups: Vec<core::SessionGroup>, search: Option<&str>) -> Vec<core::SessionGroup> {
    let needle = search.map(str::trim).filter(|value| !value.is_empty());
    let Some(needle) = needle else {
        return groups;
    };
    let needle = needle.to_lowercase();

    groups
        .into_iter()
        .filter_map(|mut group| {
            group.sessions.retain(|session| {
                session.session_id.to_lowercase().contains(&needle)
                    || session
                        .title
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&needle)
            });
            (!group.sessions.is_empty()).then_some(group)
        })
        .collect()
}

fn sort_groups(mut groups: Vec<core::SessionGroup>, sort: Option<&str>) -> Vec<core::SessionGroup> {
    for group in &mut groups {
        match sort.unwrap_or("updated_desc") {
            "updated_asc" => group.sessions.sort_by_key(|session| session.last_active_at),
            "title_asc" => group
                .sessions
                .sort_by(|left, right| session_title(left).cmp(&session_title(right))),
            "title_desc" => group
                .sessions
                .sort_by(|left, right| session_title(right).cmp(&session_title(left))),
            "size_desc" => group
                .sessions
                .sort_by_key(|session| std::cmp::Reverse(session_size_bytes(session).unwrap_or(0))),
            _ => group
                .sessions
                .sort_by_key(|session| std::cmp::Reverse(session.last_active_at)),
        }
    }
    groups
}

fn session_title(session: &core::SessionItem) -> String {
    session
        .title
        .as_deref()
        .unwrap_or("(untitled)")
        .to_lowercase()
}

fn render_session_groups(
    groups: &[core::SessionGroup],
    total: usize,
    visible_limit: usize,
    per_page: usize,
    workspace: &str,
    provider_filter: &[String],
    search: Option<&str>,
    sort: Option<&str>,
    lang: UiLanguage,
) -> Markup {
    if groups.is_empty() {
        return html! {
            div.empty-state {
                h2 { (tr(lang, "当前工作空间没有会话", "No sessions in this workspace")) }
                p { (tr(lang, "请选择其他工作空间，或确认终端智能体的会话属于这个目录。", "Choose another workspace, or confirm the terminal agent sessions belong to this directory.")) }
            }
        };
    }

    html! {
        div.stats {
            span { (tr(lang, "会话", "Sessions")) "=" (total) }
            span { (tr(lang, "终端智能体", "Terminal Agents")) "=" (groups.len()) }
            span { (tr(lang, "每智能体显示", "Per Agent")) "=" (visible_limit) }
        }
        @for group in groups {
            @let shown_count = group.sessions.len().min(visible_limit);
            details.provider-section open {
                summary {
                    span { (group.provider_name) }
                    span { (shown_count) "/" (group.sessions.len()) }
                }
                div.session-list {
                    @for session in group.sessions.iter().take(visible_limit) {
                        (render_session_row(session, lang))
                    }
                }
                @if group.sessions.len() > visible_limit {
                    div.load-more {
                        a.button data-preserve-scroll="true" href=(home_url(workspace, provider_filter, search, sort, per_page, visible_limit + per_page, lang)) {
                            (tr(lang, "加载更多", "Load More"))
                        }
                    }
                }
            }
        }
    }
}

fn render_session_row(session: &core::SessionItem, lang: UiLanguage) -> Markup {
    let title = session.title.as_deref().unwrap_or("(untitled)");
    let workspace = session.project_dir.as_deref().unwrap_or("(no directory)");
    let stats = session_row_stats(session);
    let session_href = format!(
        "/sessions/{}/{}?workspace={}&lang={}",
        session.provider_id,
        session.session_id,
        query_escape(workspace),
        lang_code(lang)
    );
    let switch_href = format!(
        "/modal/switch?from={}&session_id={}&workspace={}&lang={}",
        session.provider_id,
        query_escape(&session.session_id),
        query_escape(workspace),
        lang_code(lang)
    );
    let delete_href = format!(
        "/modal/delete/{}/{}?lang={}",
        session.provider_id,
        query_escape(&session.session_id),
        lang_code(lang)
    );

    html! {
        div.session-row {
            span.session-id { (session.session_id) }
            span.session-title { (title) }
            span.session-workspace { (workspace) }
            div.session-meta {
                span { (tr(lang, "修改", "Modified")) "=" (stats.modified_at) }
                span { (tr(lang, "轮次", "Turns")) "=" (stats.turns) }
                span { (tr(lang, "大小", "Size")) "=" (stats.size) }
            }
            div.row-actions {
                button type="button" data-modal=(switch_href) { (tr(lang, "切换", "Switch")) }
                a.button href=(session_href) { (tr(lang, "查看", "View")) }
                button type="button" data-modal=(delete_href) { (tr(lang, "删除", "Delete")) }
            }
        }
    }
}

struct SessionRowStats {
    modified_at: String,
    turns: String,
    size: String,
}

fn session_row_stats(session: &core::SessionItem) -> SessionRowStats {
    SessionRowStats {
        modified_at: format_modified_at(session.last_active_at),
        turns: format_turns(session),
        size: format_size(session_size_bytes(session)),
    }
}

fn format_modified_at(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return "-".to_string();
    };
    let datetime = if timestamp.abs() >= 1_000_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(timestamp)
    } else {
        DateTime::<Utc>::from_timestamp(timestamp, 0)
    };
    datetime
        .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_turns(session: &core::SessionItem) -> String {
    core::get_session(&session.provider_id, &session.session_id)
        .map(|loaded| loaded.messages.len().to_string())
        .unwrap_or_else(|_| "-".to_string())
}

fn session_size_bytes(session: &core::SessionItem) -> Option<u64> {
    let source_path = session.source_path.as_deref()?;
    let path = FsPath::new(source_path);
    path.is_file()
        .then(|| std::fs::metadata(path).ok().map(|metadata| metadata.len()))
        .flatten()
}

fn format_size(size: Option<u64>) -> String {
    let Some(size) = size else {
        return "-".to_string();
    };
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / 1024.0 / 1024.0)
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

async fn page_session(
    Path((provider, session_id)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
    let q = HomeQuery::from_raw(raw_query.as_deref());
    let prefs = match apply_web_preferences(&q) {
        Ok(prefs) => prefs,
        Err(e) => {
            return Html(layout("memorph - 错误", error_markup(e), UiLanguage::Zh).into_string())
        }
    };
    let lang = prefs.language;
    let session = match core::get_session(&provider, &session_id) {
        Ok(session) => session,
        Err(e) => return Html(layout("memorph - 错误", error_markup(e), lang).into_string()),
    };

    if let Some(project_dir) = session.session.project_dir.as_deref() {
        if let Err(e) = config::remember_workspace(&PathBuf::from(project_dir)) {
            return Html(layout("memorph - 错误", error_markup(e), lang).into_string());
        }
    }

    let title_text = session
        .session
        .title
        .as_deref()
        .unwrap_or("(untitled)")
        .to_string();
    let workspace = session
        .session
        .project_dir
        .as_deref()
        .or(q.workspace.as_deref())
        .unwrap_or("");
    let back_href = if workspace.is_empty() {
        format!("/?lang={}", lang_code(lang))
    } else {
        let providers = Vec::new();
        home_url(
            workspace,
            &providers,
            None,
            None,
            prefs.sessions_per_provider,
            prefs.sessions_per_provider,
            lang,
        )
    };

    let content = html! {
        section.session-header {
            div {
                p.eyebrow { (provider) }
                h1 { (title_text) }
                div.meta-line {
                    span { "id=" code { (session_id) } }
                    span { "messages=" (session.messages.len()) }
                    @if !workspace.is_empty() {
                        span { "workspace=" code { (workspace) } }
                    }
                }
            }
            div.session-actions {
                a.button href=(back_href) { (tr(lang, "返回", "Back")) }
                button type="button" data-modal=(format!("/modal/rename/{}/{}?lang={}", provider, session_id, lang_code(lang))) { (tr(lang, "重命名", "Rename")) }
                button type="button" data-modal=(format!("/modal/delete/{}/{}?lang={}", provider, session_id, lang_code(lang))) { (tr(lang, "删除", "Delete")) }
            }
        }
        div.msg-list {
            @for message in &session.messages {
                (render_message(message))
            }
        }
    };

    Html(layout(&format!("memorph - {}", title_text), content, lang).into_string())
}

fn render_message(message: &crate::model::MemorphMessage) -> Markup {
    let role = match message.role {
        MemorphRole::User => "user",
        MemorphRole::Assistant => "assistant",
        MemorphRole::Tool => "tool",
        MemorphRole::System => "system",
        MemorphRole::Developer => "developer",
    };
    let time = message.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();

    html! {
        article.msg-item {
            header.msg-header {
                span.msg-role { (role) }
                span { (time) }
            }
            div.msg-body {
                @for block in &message.content {
                    (render_content_block(block))
                }
            }
        }
    }
}

fn render_content_block(block: &ContentBlock) -> Markup {
    match block {
        ContentBlock::Text { text } => html! {
            div { (PreEscaped(simple_markdown(text))) }
        },
        ContentBlock::Thinking { thinking, .. } => html! {
            details.thinking-block {
                summary.block-label { "thinking" }
                p { (truncate_chars(thinking, 1200)) }
            }
        },
        ContentBlock::ToolUse { name, input, .. } => html! {
            details.tool-block {
                summary.block-label { "tool: " (name) }
                @if let Some(input) = input {
                    pre { code { (truncate_json(input, 1200)) } }
                }
            }
        },
        ContentBlock::ToolResult {
            content, is_error, ..
        } => html! {
            details.tool-block {
                summary.block-label {
                    @if is_error.unwrap_or(false) { "tool result: error" } @else { "tool result" }
                }
                pre { code { (truncate_chars(content, 1600)) } }
            }
        },
        ContentBlock::File { path, .. } => html! {
            div.tool-block {
                span.block-label { "file" }
                code { (path) }
            }
        },
        ContentBlock::Image { .. } => html! {
            div.tool-block {
                span.block-label { "image" }
            }
        },
    }
}

fn error_markup(error: impl std::fmt::Display) -> Markup {
    html! {
        div.empty-state {
            h2 { "操作失败" }
            p { (error) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_query_accepts_single_repeated_and_comma_providers() {
        let single = HomeQuery::from_raw(Some("provider=claude"));
        assert_eq!(normalize_provider_filter(&single.provider), vec!["claude"]);

        let repeated = HomeQuery::from_raw(Some("provider=claude&provider=codex"));
        assert_eq!(
            normalize_provider_filter(&repeated.provider),
            vec!["claude", "codex"]
        );

        let comma = HomeQuery::from_raw(Some("provider=claude,codex"));
        assert_eq!(
            normalize_provider_filter(&comma.provider),
            vec!["claude", "codex"]
        );
    }

    #[test]
    fn detects_opencode_subagent_titles() {
        assert!(is_opencode_subagent_title(Some(
            "Analyze web patterns (@Sisyphus-Junior subagent)"
        )));
        assert!(!is_opencode_subagent_title(Some(
            "Regular OpenCode session"
        )));
        assert!(!is_opencode_subagent_title(None));
    }
}
