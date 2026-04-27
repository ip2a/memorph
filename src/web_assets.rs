use axum::{http::header, response::IntoResponse};
use maud::{html, Markup, PreEscaped};

pub(crate) const CUSTOM_CSS: &str = r#"
:root {
    --paper: #fff;
    --ink: #000;
    --line: #000;
    --radius: 4px;
    --space: 16px;
    color-scheme: light;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans CJK SC", "PingFang SC", sans-serif;
    font-size: 15px;
    letter-spacing: 0;
}

* { box-sizing: border-box; }
html, body { margin: 0; min-height: 100%; background: var(--paper); color: var(--ink); }
body { line-height: 1.5; }
a { color: inherit; text-decoration: none; }
button, input, select, .button {
    font: inherit;
    color: var(--ink);
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: var(--radius);
}
button, .button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 34px;
    padding: 0 12px;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease, transform 120ms ease;
}
button:hover, .button:hover, select:hover, input:hover { background: var(--ink); color: var(--paper); }
button:active, .button:active { transform: translateY(1px); }
button:focus-visible, .button:focus-visible, input:focus-visible, select:focus-visible {
    outline: 2px solid var(--ink);
    outline-offset: 2px;
}
.invert { background: var(--ink); color: var(--paper); }
.invert:hover { background: var(--paper); color: var(--ink); }

.atomic-shell {
    width: min(1180px, calc(100vw - 32px));
    margin: 0 auto;
    position: relative;
}
.atomic-shell::before, .atomic-shell::after {
    content: "";
    position: fixed;
    pointer-events: none;
    border: 1px solid var(--line);
    border-radius: 50%;
    opacity: 0.06;
    transform: rotate(-22deg);
}
.atomic-shell::before {
    width: 560px;
    height: 180px;
    right: -180px;
    top: 92px;
}
.atomic-shell::after {
    width: 360px;
    height: 116px;
    left: -150px;
    bottom: 70px;
    transform: rotate(28deg);
}

.topbar {
    height: 64px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--line);
    margin-bottom: 24px;
}
.brand-cluster, .brand, .top-actions { display: flex; align-items: center; gap: 10px; }
.brand { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-weight: 700; }
.settings-entry { min-width: 72px; }
.version { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; }
.github-link, .icon-button {
    width: 34px;
    height: 34px;
    display: inline-grid;
    place-items: center;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    transition: background 120ms ease, color 120ms ease, transform 120ms ease;
    padding: 0;
}
.github-link:hover, .icon-button:hover { background: var(--ink); color: var(--paper); }
.github-link:active, .icon-button:active { transform: translateY(1px); }
.github-link:focus-visible, .icon-button:focus-visible { outline: 2px solid var(--ink); outline-offset: 2px; }
.github-link svg, .icon-button svg { width: 16px; height: 16px; }
.lang-switch {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.lang-switch select { height: 34px; padding: 0 8px; }

.ascii-banner {
    display: grid;
    place-items: center;
    margin: 6px 0 24px;
    padding: 20px 0;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    overflow: hidden;
}
.ascii-banner pre {
    margin: 0;
    border: 0;
    padding: 0;
    overflow: visible;
    max-width: none;
    text-align: center;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: clamp(6px, 0.88vw, 12px);
    line-height: 1;
    font-weight: 900;
    letter-spacing: 0.02em;
    white-space: pre;
}
.workspace-panel {
    display: grid;
    grid-template-columns: minmax(260px, 0.72fr) minmax(420px, 1.28fr);
    gap: 18px;
    align-items: stretch;
    margin: 18px 0 22px;
    padding: 18px 0;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
}
.workspace-hero {
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 12px;
    min-height: 118px;
    padding-right: 18px;
    border-right: 1px solid var(--line);
}
.eyebrow {
    margin: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    text-transform: uppercase;
}
h1, h2, h3, p { margin: 0; }
h1 {
    font-size: clamp(26px, 4vw, 48px);
    line-height: 1.05;
    font-weight: 750;
}
.workspace-path {
    overflow-wrap: anywhere;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
}

.filter-panel {
    display: grid;
    gap: 12px;
    align-items: stretch;
    margin: 0;
}
.filter-row {
    display: grid;
    grid-template-columns: 1fr;
    gap: 10px;
    align-items: end;
}
.filter-row-compact { grid-template-columns: minmax(260px, 1fr) minmax(130px, 0.32fr) auto auto; }
.filter-row-main {
    grid-template-columns: minmax(360px, 1fr) minmax(260px, auto);
}
.workspace-combo {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 8px;
}
.workspace-combo button { min-width: 64px; }
.field { display: grid; gap: 6px; }
.field-wide { min-width: 0; }
.field-number { min-width: 96px; }
.field label {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    text-transform: uppercase;
}
.field input, .field select {
    width: 100%;
    height: 36px;
    padding: 0 10px;
}
.agent-field { min-width: 0; }
.agent-picker {
    min-height: 36px;
    display: flex;
    flex-wrap: nowrap;
    gap: 6px;
    align-items: center;
}
.agent-pill {
    display: inline-flex;
    align-items: center;
    cursor: pointer;
}
.agent-pill input {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    opacity: 0;
    pointer-events: none;
}
.agent-pill span {
    min-height: 30px;
    display: inline-flex;
    align-items: center;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 999px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.agent-pill input:checked + span {
    background: var(--ink);
    color: var(--paper);
}
.agent-pill input:focus-visible + span {
    outline: 2px solid var(--ink);
    outline-offset: 2px;
}
.load-more {
    display: flex;
    justify-content: center;
    padding: 14px 0 18px;
}
.page-loading main, .page-loading .topbar {
    opacity: 0.45;
    pointer-events: none;
}
.loading-layer {
    position: fixed;
    inset: 0;
    display: none;
    place-items: center;
    z-index: 20;
    background: rgba(255, 255, 255, 0.78);
}
.page-loading .loading-layer { display: grid; }
.loading-card {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--line);
    background: var(--paper);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.loading-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--line);
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 700ms linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

.stats {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    margin-bottom: 14px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.provider-section {
    border-top: 1px solid var(--line);
}
.provider-section summary {
    min-height: 44px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    cursor: pointer;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-weight: 700;
}
.session-list { display: grid; border-top: 1px solid var(--line); }
.session-row {
    display: grid;
    grid-template-columns: minmax(150px, 0.65fr) minmax(210px, 1.15fr) minmax(220px, 1.25fr) minmax(240px, 1.2fr) auto;
    gap: 14px;
    align-items: center;
    min-height: 54px;
    border-bottom: 1px solid var(--line);
    padding: 8px 0;
}
.session-row:hover { background: var(--ink); color: var(--paper); }
.session-row > * { min-width: 0; }
.row-actions {
    display: flex;
    justify-content: end;
    gap: 6px;
    flex-wrap: wrap;
}
.row-actions .button, .row-actions button {
    min-height: 30px;
    padding: 0 9px;
    font-size: 12px;
}
.session-id, code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.session-id {
    overflow-wrap: anywhere;
    word-break: break-word;
    line-height: 1.25;
}
.session-title {
    font-weight: 650;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.session-workspace {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.session-meta {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.session-meta span { white-space: nowrap; }
.empty-state {
    border: 1px solid var(--line);
    padding: 32px;
    text-align: center;
}

.session-header {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 16px;
    align-items: start;
    padding-bottom: 18px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 18px;
}
.session-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: end; }
.meta-line {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 10px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.msg-list { display: grid; gap: 10px; }
.msg-item { border: 1px solid var(--line); }
.msg-header {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--line);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.msg-role { font-weight: 700; text-transform: uppercase; }
.msg-body { padding: 12px; overflow-wrap: anywhere; }
pre {
    overflow: auto;
    border: 1px solid var(--line);
    padding: 10px;
    margin: 8px 0;
    background: var(--paper);
    color: var(--ink);
}
.tool-block, .thinking-block {
    border-left: 4px solid var(--line);
    padding: 8px 10px;
    margin: 8px 0;
}
.block-label {
    display: block;
    margin-bottom: 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    font-weight: 700;
}

dialog {
    width: min(480px, calc(100vw - 32px));
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0;
    color: var(--ink);
    background: var(--paper);
}
.workspace-history-modal { width: min(640px, calc(100vw - 32px)); }
.settings-modal { width: min(560px, calc(100vw - 32px)); }
dialog::backdrop { background: #000; opacity: 0.72; }
dialog article { padding: 18px; }
dialog header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
}
dialog form { display: grid; gap: 12px; }
dialog footer { display: flex; justify-content: end; gap: 8px; margin-top: 16px; }
.switch-result-modal { width: min(640px, calc(100vw - 32px)); }
.success-callout {
    display: grid;
    gap: 4px;
    padding: 10px 0 12px;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
}
.result-grid {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 8px 12px;
    margin-top: 14px;
    align-items: start;
}
.result-grid > span {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    text-transform: uppercase;
}
.result-grid code, .verify-block code { overflow-wrap: anywhere; }
.verify-block { margin-top: 14px; }
.verify-block pre { margin-bottom: 0; }
.settings-list { display: grid; border-top: 1px solid var(--line); }
.settings-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 14px;
    align-items: center;
    min-height: 54px;
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
}
.settings-copy { display: grid; gap: 2px; min-width: 0; }
.settings-copy strong { font-size: 14px; }
.settings-copy span {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
}
.settings-row select { min-width: 136px; height: 34px; padding: 0 8px; }
.settings-row input[type="number"] { width: 110px; height: 34px; padding: 0 8px; }
.settings-check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    cursor: pointer;
    white-space: nowrap;
}
.settings-check:hover { background: var(--ink); color: var(--paper); }
.settings-check input { width: 16px; height: 16px; margin: 0; }
.modal-subtitle {
    margin-top: 4px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
}
.workspace-history-list {
    display: grid;
    gap: 8px;
    max-height: min(52vh, 420px);
    overflow: auto;
}
.workspace-history-item {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px 12px;
    width: 100%;
    min-height: auto;
    padding: 10px;
    text-align: left;
}
.workspace-history-item code {
    grid-column: 1 / -1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.86;
}
.workspace-history-name { font-weight: 700; }
.workspace-history-time {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    white-space: nowrap;
}

@media (max-width: 780px) {
    .atomic-shell { width: min(100vw - 20px, 1180px); }
    .workspace-panel { grid-template-columns: 1fr; gap: 14px; }
    .workspace-hero { min-height: auto; padding-right: 0; padding-bottom: 14px; border-right: 0; border-bottom: 1px solid var(--line); }
    .filter-panel, .filter-row-main, .filter-row-compact, .workspace-combo, .session-header, .settings-row { grid-template-columns: 1fr; }
    .agent-picker { flex-wrap: wrap; }
    .session-row { grid-template-columns: 1fr; gap: 4px; padding: 12px 0; }
    .row-actions { justify-content: start; margin-top: 8px; }
    .session-title, .session-workspace { white-space: normal; }
    .topbar { height: auto; padding: 14px 0; gap: 12px; align-items: flex-start; }
    .brand-cluster { flex-wrap: wrap; }
    .top-actions { flex-wrap: wrap; justify-content: end; }
}
"#;

pub(crate) const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#fff"/><circle cx="32" cy="32" r="24" fill="none" stroke="#000" stroke-width="4"/><ellipse cx="32" cy="32" rx="27" ry="9" fill="none" stroke="#000" stroke-width="3" transform="rotate(-25 32 32)"/><circle cx="32" cy="32" r="4" fill="#000"/></svg>"##;

pub(crate) const MEMORPH_ASCII: &str = r#"███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████   ██  ██   ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██   ██  ██   ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██   ██  ██   ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██"#;

pub(crate) fn modal_script() -> Markup {
    html! {
        script {
            (PreEscaped(r#"
async function loadModal(url) {
    setLoading(true);
    try {
        const response = await fetch(url);
        if (!response.ok) throw new Error(await response.text());
        mountModal(await response.text());
    } finally {
        setLoading(false);
    }
}

async function refreshMain() {
    setLoading(true);
    try {
        const response = await fetch(window.location.href, { headers: { 'X-Memorph-Partial': 'main' } });
        if (!response.ok) throw new Error(await response.text());
        const html = await response.text();
        const next = new DOMParser().parseFromString(html, 'text/html');
        const currentMain = document.querySelector('main');
        const nextMain = next.querySelector('main');
        if (currentMain && nextMain) currentMain.replaceWith(nextMain);
    } finally {
        setLoading(false);
    }
}

function afterDeleteRefresh() {
    if (window.location.pathname.startsWith('/sessions/')) {
        goUrl('/' + window.location.search);
        return;
    }
    refreshMain();
}

function setLoading(active) {
    document.body.classList.toggle('page-loading', Boolean(active));
}

function mountModal(html) {
    document.getElementById('modal-container').innerHTML = html;
    const dialog = document.querySelector('#modal-container dialog');
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();

    const target = dialog.querySelector('input:not([type="hidden"]), textarea, select');
    if (target) requestAnimationFrame(() => target.focus());
}

function closeModal() {
    const dialog = document.querySelector('#modal-container dialog');
    if (dialog) dialog.close();
    document.getElementById('modal-container').innerHTML = '';
}

function setLanguage(lang) {
    const url = new URL(window.location.href);
    url.searchParams.set('lang', lang);
    setLoading(true);
    window.location.href = url.toString();
}

function goWorkspace(path) {
    if (!path) return;
    const url = new URL(window.location.href);
    url.searchParams.set('workspace', path);
    url.searchParams.delete('visible');
    goUrl(url.toString());
}

function goUrl(url) {
    setLoading(true);
    window.location.href = url;
}

function submitWithLoading(form) {
    if (!form) return;
    setLoading(true);
    form.submit();
}

window.addEventListener('DOMContentLoaded', function() {
    const stored = sessionStorage.getItem('memorph-scroll-y');
    if (!stored) return;
    sessionStorage.removeItem('memorph-scroll-y');
    const y = Number(stored);
    if (Number.isFinite(y)) requestAnimationFrame(() => window.scrollTo(0, y));
});

document.addEventListener('click', function(event) {
    const preserveScroll = event.target.closest('[data-preserve-scroll]');
    if (preserveScroll) {
        sessionStorage.setItem('memorph-scroll-y', String(window.scrollY));
        setLoading(true);
    }

    const trigger = event.target.closest('[data-modal]');
    if (trigger) {
        event.preventDefault();
        loadModal(trigger.getAttribute('data-modal')).catch((error) => {
            mountModal('<dialog><article><h3>错误</h3><p></p><button onclick="closeModal()">关闭</button></article></dialog>');
            const message = document.querySelector('#modal-container p');
            if (message) message.textContent = error.message;
        });
        return;
    }

    const link = event.target.closest('a[href]');
    if (!link || link.target || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const href = link.getAttribute('href');
    if (!href || href.startsWith('#') || href.startsWith('javascript:')) return;
    const url = new URL(href, window.location.href);
    if (url.origin !== window.location.origin) return;
    setLoading(true);
});

document.addEventListener('submit', function(event) {
    const form = event.target.closest('[data-modal-form]');
    if (!form) return;
    event.preventDefault();
    const params = new URLSearchParams(new FormData(form));
    loadModal(form.action + '?' + params.toString()).catch((error) => {
        mountModal('<dialog><article><h3>错误</h3><p></p><button onclick="closeModal()">关闭</button></article></dialog>');
        const message = document.querySelector('#modal-container p');
        if (message) message.textContent = error.message;
    });
});

document.addEventListener('submit', function(event) {
    const form = event.target.closest('form');
    if (!form || form.matches('[data-modal-form]')) return;
    setLoading(true);
});
"#))
        }
    }
}

pub(crate) async fn serve_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        CUSTOM_CSS,
    )
}

pub(crate) async fn serve_favicon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], FAVICON_SVG)
}
