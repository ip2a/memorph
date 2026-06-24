let toastTimer = 0;

function pageTitle(state, t) {
  switch (state.route.name) {
    case "manager":
      return t("managerTitle");
    case "compression":
      return t("compressionTitle");
    case "agents":
      return t("agentManagementTitle");
    case "hooks":
      return t("hooksTitle");
    case "shared-list":
      return t("sharedTitle");
    case "shared-detail":
      return t("sharedTitle");
    case "session":
      return state.session?.view?.provider_name || t("details");
    default:
      return "";
  }
}

export function renderTopbarContext(state, t, escapeHtml) {
  const label = pageTitle(state, t);
  if (!label) return "";
  return `
    <span class="topbar-divider"></span>
    <span class="topbar-page">${escapeHtml(label)}</span>`;
}

export function renderAppShell({
  state,
  t,
  escapeHtml,
  renderPage,
  renderLoading,
  renderToasts,
  renderTopbarContext,
  githubIcon,
}) {
  if (!state.meta) {
    return `
      <div class="app-shell">
        <nav class="topbar">
          <div class="brand">memorph</div>
        </nav>
        <main class="app-main">${renderLoading()}</main>
      </div>`;
  }

  return `
    <div class="app-shell ${state.route.name === "manager" || state.route.name === "compression" || state.route.name === "agents" || state.route.name === "hooks" ? "manager-shell" : ""}">
      <nav class="topbar">
        <div class="brand-cluster">
          <a class="brand" href="/" data-nav="/">memorph</a>
          ${renderTopbarContext()}
        </div>
        <div class="top-actions">
          ${state.route.name === "home" ? "" : `<a class="button" href="/" data-nav="/">${state.route.name === "session" ? t("back") : t("openHome")}</a>`}
          <button type="button" data-action="open-workspace-switch">${t("switchWorkspace")}</button>
          ${state.route.name === "hooks" ? "" : `<a class="button" href="/hooks" data-nav="/hooks">${t("hooks")}</a>`}
          ${state.route.name === "agents" ? "" : `<a class="button" href="/agents" data-nav="/agents">${t("agentManagement")}</a>`}
          ${state.route.name === "manager" ? `<button type="button" data-action="open-compression">${t("compressSessions")}</button><a class="button" href="/shared" data-nav="/shared">${t("sharedGroups")}</a><button type="button" data-action="open-import">${t("importSession")}</button>` : `<a class="button" href="/manager" data-nav="/manager">${t("manage")}</a>`}
          <button type="button" data-action="open-settings">${t("settings")}</button>
          <a class="icon-button" href="https://github.com/ip2a/memorph" target="_blank" rel="noopener noreferrer" data-action="open-external" data-url="https://github.com/ip2a/memorph" aria-label="GitHub repository" title="GitHub">
            ${githubIcon()}
          </a>
        </div>
      </nav>
      <main class="app-main">${renderPage()}</main>
    </div>
    ${renderLoading()}
    ${renderToasts()}`;
}

export function renderModalMarkup(state, t, escapeHtml, escapeAttr) {
  if (!state.modal) {
    return "";
  }
  const modal = state.modal;
  const formOpen = modal.kind === "form";
  const actionsInHead = formOpen && modal.actionsInHead;
  const extraActionButtons = (modal.extraActions || [])
    .map(
      (item) =>
        `<button type="submit" name="action" value="${escapeAttr(item.action || item.label)}" class="${escapeAttr(item.className || "invert")}">${escapeHtml(item.label)}</button>`
    )
    .join("");
  return `
    <div class="overlay">
      <div class="modal-card ${modal.className || ""}">
        ${formOpen ? `<form data-submit="${escapeAttr(modal.submit)}" class="modal-stack">` : ""}
        <div class="modal-head">
          <div>
            <h3>${escapeHtml(modal.title)}</h3>
            ${modal.subtitle ? `<p class="muted">${escapeHtml(modal.subtitle)}</p>` : ""}
          </div>
          ${
            actionsInHead
              ? `<div class="modal-head-actions">
                  <button type="button" data-action="close-modal">${t("cancel")}</button>
                  <button type="submit" class="${modal.submitClass || "invert"}">${escapeHtml(modal.submitLabel || t("save"))}</button>
                </div>`
              : `<button type="button" class="text-close-button ghost" data-action="close-modal">${t("close")}</button>`
          }
        </div>
        ${
          formOpen
            ? `${modal.body}
              ${
                actionsInHead
                  ? ""
                  : `<div class="modal-actions">
                      <button type="button" data-action="close-modal">${t("cancel")}</button>
                      <button type="submit" name="action" value="copy" class="${modal.submitClass || "invert"}">${escapeHtml(modal.submitLabel || t("save"))}</button>
                      ${extraActionButtons}
                    </div>`
              }
            </form>`
            : modal.body
        }
      </div>
    </div>`;
}

export function renderLoadingMarkup(state, t, escapeHtml) {
  const info = state.loadingInfo || {};
  const progress = typeof info.progress === "number" ? Math.max(0, Math.min(1, info.progress)) : null;
  return `
    <div class="loading-layer ${state.loading ? "active" : ""}">
      <div class="loading-card">
        <span class="loading-spinner"></span>
        <div class="loading-copy">
          <strong>${escapeHtml(info.label || t("loading"))}</strong>
          ${info.detail ? `<span>${escapeHtml(info.detail)}</span>` : ""}
        </div>
        ${
          progress === null
            ? `<div class="loading-bar indeterminate"><span></span></div>`
            : `<div class="loading-bar"><span style="width: ${Math.round(progress * 100)}%"></span></div>
               <span class="loading-percent">${Math.round(progress * 100)}%</span>`
        }
      </div>
    </div>`;
}

export function renderToastsMarkup(state, t, escapeHtml, escapeAttr) {
  return `
    <div class="toast-stack">
      ${state.toasts
        .map(
          (item, index) => `
          <div class="toast ${item.error ? "error" : ""}">
            <div>
              <h4>${escapeHtml(item.title)}</h4>
              <p>${escapeHtml(item.message)}</p>
            </div>
            <button type="button" class="toast-close" data-action="close-toast" data-toast-index="${index}" aria-label="${escapeAttr(
              t("close")
            )}">${t("close")}</button>
          </div>`
        )
        .join("")}
    </div>`;
}

export function closeToast(state, index, rerender) {
  if (!Number.isInteger(index)) return;
  state.toasts = state.toasts.filter((_, itemIndex) => itemIndex !== index);
  rerender();
}

export function closeModal(state, rerender) {
  state.modal = null;
  rerender();
}

export function githubIcon() {
  return `<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.6 7.6 0 0 1 8 3.86c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>`;
}

export function toast(state, title, message, error, rerender) {
  state.toasts = [...state.toasts, { title, message, error }].slice(-4);
  rerender();
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    state.toasts = state.toasts.slice(-1);
    rerender();
  }, 3200);
}

export function fatal(appEl, error, t, escapeHtml) {
  appEl.innerHTML = `<div class="app-shell"><div class="empty-state"><h2>${escapeHtml(t("error"))}</h2><p>${escapeHtml(
    error.message || String(error)
  )}</p></div></div>`;
}

export function setLoading(state, active, info, rerender) {
  state.loading += active ? 1 : -1;
  if (state.loading < 0) state.loading = 0;
  if (active && info) state.loadingInfo = info;
  if (!active && state.loading === 0) state.loadingInfo = null;
  rerender();
}

export function updateLoading(state, info, rerender) {
  state.loadingInfo = { ...(state.loadingInfo || {}), ...info };
  rerender();
}
