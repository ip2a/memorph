import { routeShellClass, routeTitleKey } from "./router.js";

let toastId = 0;
const toastDismissTimers = new Map();

function pageTitle(state, providers, t) {
  if (state.route.name === "session") {
    return providers.displayName(state.session?.view?.provider_id) || t("details");
  }
  const titleKey = routeTitleKey(state.route);
  return titleKey ? t(titleKey) : "";
}

export function renderTopbarContext(state, providers, t, escapeHtml) {
  const label = pageTitle(state, providers, t);
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
    <div class="app-shell ${routeShellClass(state.route)}">
      <nav class="topbar">
        <div class="brand-cluster">
          <a class="brand" href="/" data-nav="/">memorph</a>
          ${renderTopbarContext()}
        </div>
        <div class="top-actions">
          ${state.route.name === "home" ? "" : `<button type="button" class="topbar-back" data-action="go-back">${t("back")}</button>`}
          <button type="button" data-action="open-workspace-switch">${t("switchWorkspace")}</button>
          ${state.route.name === "hooks" ? "" : `<a class="button" href="/hooks" data-nav="/hooks">${t("hooks")}</a>`}
          ${state.route.name === "agents" ? "" : `<a class="button" href="/agents" data-nav="/agents">${t("agentManagement")}</a>`}
          ${state.route.name === "manager" ? `<button type="button" data-action="open-compression">${t("compressSessions")}</button><a class="button" href="/sync" data-nav="/sync">${t("syncGroups")}</a><button type="button" data-action="open-import">${t("importSession")}</button>` : `<a class="button" href="/manager" data-nav="/manager">${t("manage")}</a>`}
          <button type="button" data-action="open-settings">${t("settings")}</button>
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
    <div class="toast-stack" aria-live="polite" aria-atomic="false">
      ${state.toasts
        .map(
          (item) => `
          <div class="toast ${item.error ? "error" : ""} ${item.closing ? "closing" : ""}">
            <div class="toast-content">
              <h4>${escapeHtml(item.title)}</h4>
              <p>${escapeHtml(item.message)}</p>
            </div>
            <button type="button" class="toast-close" data-action="close-toast" data-toast-id="${escapeAttr(
              String(item.id)
            )}" aria-label="${escapeAttr(t("close"))}">×</button>
          </div>`
        )
        .join("")}
    </div>`;
}

function removeToast(state, id, rerender) {
  if (!Number.isInteger(id)) return;
  const nextToasts = state.toasts.filter((item) => item.id !== id);
  if (nextToasts.length === state.toasts.length) return;
  state.toasts = nextToasts;
  window.clearTimeout(toastDismissTimers.get(id));
  toastDismissTimers.delete(id);
  rerender();
}

function dismissToast(state, id, rerender) {
  if (!Number.isInteger(id)) return;
  let didUpdate = false;
  state.toasts = state.toasts.map((item) => {
    if (item.id !== id || item.closing) return item;
    didUpdate = true;
    return { ...item, closing: true };
  });
  if (!didUpdate) return;
  rerender();
  window.clearTimeout(toastDismissTimers.get(id));
  toastDismissTimers.set(
    id,
    window.setTimeout(() => removeToast(state, id, rerender), 180)
  );
}

export function closeToast(state, id, rerender) {
  dismissToast(state, id, rerender);
}

export function closeModal(state, rerender) {
  state.modal = null;
  rerender();
}

export function toast(state, title, message, error, rerender) {
  const item = { id: ++toastId, title, message, error, closing: false };
  const removedItems = Math.max(0, state.toasts.length + 1 - 4);
  state.toasts.slice(0, removedItems).forEach((toastItem) => {
    window.clearTimeout(toastDismissTimers.get(toastItem.id));
    toastDismissTimers.delete(toastItem.id);
  });
  state.toasts = [...state.toasts, item].slice(-4);
  rerender();
  toastDismissTimers.set(
    item.id,
    window.setTimeout(() => dismissToast(state, item.id, rerender), 3200)
  );
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
