const ASCII = `███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████  ██    ██  ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██  ██    ██  ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██  ██    ██  ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██`;

let I18N = { zh: {}, en: {} };

const state = {
  meta: null,
  route: parseRoute(window.location.pathname),
  loading: 0,
  ui: {
    homeProviderVisibleCount: null,
  },
  home: {
    workspace: "",
    providers: [],
    search: "",
    sort: "recent",
    visible: 12,
    groups: [],
    sharedGroups: [],
  },
  session: null,
  sharedDetail: null,
  modal: null,
  toasts: [],
};

const appEl = document.getElementById("app");
const modalRoot = document.getElementById("modal-root");

window.addEventListener("popstate", () => {
  state.route = parseRoute(window.location.pathname);
  void loadRoute();
});

window.addEventListener("resize", () => {
  scheduleHomeProviderLayout();
});

document.addEventListener("click", (event) => {
  const nav = event.target.closest("[data-nav]");
  if (nav) {
    event.preventDefault();
    navigate(nav.dataset.nav);
    return;
  }

  const action = event.target.closest("[data-action]");
  if (!action) return;
  event.preventDefault();
  void handleAction(action.dataset.action, action.dataset, action);
});

document.addEventListener("submit", (event) => {
  const form = event.target.closest("form[data-submit]");
  if (!form) return;
  event.preventDefault();
  void handleSubmit(form.dataset.submit, new FormData(form));
});

document.addEventListener("change", (event) => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return;
  if (target.dataset.role === "lang-switch") {
    updateLanguage(target.value);
  }
  if (target.dataset.role === "workspace-switch") {
    void setWorkspace(target.value);
  }
  if (target.dataset.role === "provider-toggle") {
    toggleProvider(target.value, target.checked);
  }
  if (target.dataset.role === "home-search") {
    state.home.search = target.value;
    render();
  }
  if (target.dataset.role === "select-all-manager") {
    document
      .querySelectorAll('input[name="manager_item"]')
      .forEach((el) => (el.checked = target.checked));
  }
});

void bootstrap();

async function bootstrap() {
  setLoading(true);
  try {
    await loadI18n();
    state.meta = await api("/api/v1/meta");
    setDocumentLanguage();
    state.home.visible = state.meta.settings.sessions_per_provider;
    if (!state.home.workspace) {
      state.home.workspace =
        state.meta.selected_workspace || state.meta.workspaces[0]?.path || "";
    }
    if (state.home.workspace) {
      state.home.providers = await api(
        `/api/v1/workspaces/providers?workspace=${encodeURIComponent(state.home.workspace)}`
      );
    } else {
      state.home.providers = state.meta.settings.primary_agents.length
        ? [...state.meta.settings.primary_agents]
        : state.meta.providers.map((item) => item.id);
    }
    await loadRoute();
  } catch (error) {
    fatal(error);
  } finally {
    setLoading(false);
  }
}

async function loadRoute() {
  render();
  const route = state.route;
  setLoading(true);
  try {
    if (route.name === "home") {
      await loadHome();
    } else if (route.name === "session") {
      const detail = await api(
        `/api/v1/sessions/${encodeURIComponent(route.provider)}/${encodeURIComponent(route.sessionId)}`
      );
      state.session = detail;
      const detailWorkspace = detail.view?.workspace_dir;
      if (detailWorkspace && detailWorkspace !== state.home.workspace) {
        state.home.workspace = detailWorkspace;
        state.home.providers = await api(
          `/api/v1/workspaces/providers?workspace=${encodeURIComponent(detailWorkspace)}`
        );
      }
      state.home.sharedGroups = await api("/api/v1/share/status");
    } else if (route.name === "shared-list") {
      state.home.sharedGroups = await api("/api/v1/share/status");
    } else if (route.name === "shared-detail") {
      state.home.sharedGroups = await api("/api/v1/share/status");
      state.sharedDetail = await api(
        `/api/v1/share/status?group_id=${encodeURIComponent(route.groupId)}`
      );
    }
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    setLoading(false);
    render();
  }
}

async function loadHome() {
  if (!state.home.workspace) {
    state.home.groups = [];
    state.home.sharedGroups = await api("/api/v1/share/status");
    return;
  }
  const params = new URLSearchParams({
    workspace: state.home.workspace,
    all: "false",
    provider: state.home.providers.join(","),
  });
  state.home.groups = await api(`/api/v1/sessions?${params.toString()}`);
  state.home.sharedGroups = await api("/api/v1/share/status");
}

function navigate(path) {
  const samePath = window.location.pathname === path;
  state.modal = null;
  if (!samePath) {
    history.pushState({}, "", path);
  }
  state.route = parseRoute(path);
  void loadRoute();
}

function replacePath(path) {
  history.replaceState({}, "", path);
  state.route = parseRoute(path);
}

async function updateLanguage(language) {
  try {
    state.meta.settings.language = language;
    await saveSettings({
      ...state.meta.settings,
      language,
    });
  } catch (error) {
    toast(t("error"), error.message, true);
  }
}

async function setWorkspace(workspace) {
  state.home.workspace = workspace;
  if (!workspace) {
    state.home.providers = [];
    await loadHome();
    await refreshWorkspaceMeta();
    render();
    return;
  }
  state.home.providers = await api(
    `/api/v1/workspaces/providers?workspace=${encodeURIComponent(workspace)}`
  );
  await loadHome();
  await refreshWorkspaceMeta();
  render();
}

async function refreshWorkspaceMeta() {
  state.meta.workspaces = await api("/api/v1/workspaces");
  state.meta.selected_workspace = state.home.workspace || null;
}

function toggleProvider(provider, checked) {
  const next = new Set(state.home.providers);
  if (checked) next.add(provider);
  else next.delete(provider);
  state.home.providers = [...next];
  void persistProvidersAndReload();
}

async function persistProvidersAndReload() {
  if (!state.home.workspace) return;
  try {
    await api("/api/v1/workspaces/providers", {
      method: "PUT",
      body: {
        workspace: state.home.workspace,
        providers: state.home.providers,
      },
    });
    await loadHome();
    render();
  } catch (error) {
    toast(t("error"), error.message, true);
  }
}

async function handleAction(action, data, trigger = null) {
  switch (action) {
    case "open-settings":
      openSettingsModal();
      break;
    case "open-manager":
      openManagerModal();
      break;
    case "open-import":
      openImportModal();
      break;
    case "open-workspace-switch":
      openWorkspaceSwitchModal();
      break;
    case "open-sort-options":
      openSortOptionsModal();
      break;
    case "open-agent-filter":
      openAgentFilterModal();
      break;
    case "open-workspace-history":
      openWorkspaceHistoryModal();
      break;
    case "open-switch":
      openSwitchModal(data.provider, data.sessionId, data.workspace || state.home.workspace);
      break;
    case "open-export":
      openExportModal(data.provider, data.sessionId);
      break;
    case "open-rename":
      openRenameModal(data.provider, data.sessionId, data.title || "");
      break;
    case "open-delete":
      openDeleteModal(data.provider, data.sessionId);
      break;
    case "open-share-create":
      openShareCreateModal(data.provider, data.sessionId, data.title || "");
      break;
    case "open-shared-rename":
      openSharedRenameModal(data.groupId, data.title || "");
      break;
    case "open-shared-remove":
      openSharedRemoveModal(data.groupId);
      break;
    case "open-shared-bind":
      openSharedBindModal(data.groupId);
      break;
    case "run-sync-latest":
      await runSharedSync(data.groupId);
      break;
    case "open-sync-from":
      openPushSyncModal(data.groupId, data.holdingId, data.provider || "", data.sessionId || "");
      break;
    case "open-unbind":
      openUnbindModal(data.groupId, data.holdingId, data.provider || "", data.sessionId || "");
      break;
    case "go-home":
      navigate("/");
      break;
    case "pick-workspace":
      closeModal();
      await setWorkspace(data.workspace || "");
      break;
    case "delete-workspace-history":
      await deleteWorkspaceHistory(data.workspace || "");
      break;
    case "browse-folder":
      await browseFolderForField(trigger);
      break;
    case "close-modal":
      closeModal();
      break;
    default:
      break;
  }
}

async function handleSubmit(kind, formData) {
  try {
    switch (kind) {
      case "home-filters":
        {
          const workspace = String(formData.get("workspace") || "").trim();
          const workspaceChanged = workspace !== state.home.workspace;
          state.home.search = String(formData.get("search") || "");
          state.home.sort = String(formData.get("sort") || "recent");
          state.home.visible = Number(formData.get("visible") || state.meta.settings.sessions_per_provider);
          if (workspaceChanged) {
            await setWorkspace(workspace);
          } else {
            render();
          }
        }
        break;
      case "home-search":
        state.home.search = String(formData.get("search") || "");
        render();
        break;
      case "workspace-switch":
        await setWorkspace(String(formData.get("workspace") || "").trim());
        closeModal();
        break;
      case "home-list-options":
        state.home.sort = String(formData.get("sort") || "recent");
        state.home.visible = Number(formData.get("visible") || state.meta.settings.sessions_per_provider);
        closeModal();
        break;
      case "import-session":
        await runImport(formData);
        break;
      case "switch-session":
        await runSwitch(formData);
        break;
      case "export-session":
        await runExport(formData);
        break;
      case "rename-session":
        await runRename(formData);
        break;
      case "delete-session":
        await runDelete(formData);
        break;
      case "create-shared":
        await runShareCreate(formData);
        break;
      case "rename-shared":
        await runSharedRename(formData);
        break;
      case "remove-shared":
        await runSharedRemove(formData);
        break;
      case "bind-shared":
        await runSharedBind(formData);
        break;
      case "sync-from-shared":
        await runSharedSync(String(formData.get("group_id")), String(formData.get("holding_id")));
        break;
      case "unbind-shared":
        await runUnbind(String(formData.get("group_id")), String(formData.get("holding_id")));
        break;
      case "save-settings":
        await runSaveSettings(formData);
        break;
      case "preview-manager":
        await runManagerPreview(formData);
        break;
      case "clean-manager":
        await runManagerClean(formData);
        break;
      case "backup-manager":
        await runManagerBackup(formData);
        break;
      default:
        break;
    }
  } catch (error) {
    toast(t("error"), error.message, true);
  }
}

function openImportModal() {
  state.modal = {
    kind: "form",
    title: t("import"),
    submit: "import-session",
    body: `
      <div class="stack">
        <label class="field">
          <span>${t("targetProvider")}</span>
          <select name="provider">${providerOptions()}</select>
        </label>
        <label class="field">
          <span>${t("fileOrId")}</span>
          <input name="file_or_id" required placeholder="${escapeAttr(t("fileOrIdPlaceholder"))}">
        </label>
        ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"), state.home.workspace)}
      </div>
      ${renderWorkspaceDatalist()}`,
    submitLabel: t("import"),
  };
  render();
}

function openWorkspaceSwitchModal() {
  const items = (state.meta.workspaces || [])
    .map(
      (item) => `
      <div class="workspace-option workspace-switch-item">
        <button type="button" class="workspace-option-main" data-action="pick-workspace" data-workspace="${escapeAttr(item.path)}">
          <div class="workspace-option-head">
            <strong>${escapeHtml(workspaceName(item.path))}</strong>
            <span class="workspace-time">${escapeHtml(formatDate(item.last_viewed_at))}</span>
          </div>
          <div class="workspace-option-path">${escapeHtml(item.path)}</div>
        </button>
        <button type="button" class="ghost workspace-delete" data-action="delete-workspace-history" data-workspace="${escapeAttr(item.path)}">${t("remove")}</button>
      </div>`
    )
    .join("");
  state.modal = {
    kind: "form",
    title: t("switchWorkspace"),
    submit: "workspace-switch",
    body: `
      <div class="stack">
        <label class="field">
          <span>${t("workspacePath")}</span>
          <div class="path-picker">
            <input name="workspace" list="known-workspaces" value="${escapeAttr(state.home.workspace || "")}" placeholder="${escapeAttr(
              state.meta?.workspaces?.[0]?.path || ""
            )}">
            <button type="button" class="ghost" data-action="browse-folder" data-target-field="workspace">${t("browse")}</button>
          </div>
          <small class="muted">${t("workspaceFieldHint")}</small>
        </label>
        <div class="workspace-list workspace-switch-list">${items || `<div class="empty-state">${t("noWorkspace")}</div>`}</div>
      </div>
      ${renderWorkspaceDatalist()}`,
    submitLabel: t("go"),
  };
  render();
}

async function deleteWorkspaceHistory(workspace) {
  if (!workspace) return;
  const workspaces = await api("/api/v1/workspaces/history", {
    method: "DELETE",
    body: { workspace },
  });
  state.meta.workspaces = workspaces;
  if (state.meta.selected_workspace === workspace) {
    state.meta.selected_workspace = null;
  }
  openWorkspaceSwitchModal();
}

function openWorkspaceHistoryModal() {
  const items = (state.meta.workspaces || [])
    .map(
      (item) => `
      <button type="button" class="workspace-option" data-action="pick-workspace" data-workspace="${escapeAttr(item.path)}">
        <div class="workspace-option-head">
          <strong>${escapeHtml(workspaceName(item.path))}</strong>
          <span class="workspace-time">${escapeHtml(formatDate(item.last_viewed_at))}</span>
        </div>
        <div class="path-line">${escapeHtml(item.path)}</div>
      </button>`
    )
    .join("");
  state.modal = {
    kind: "custom",
    title: t("workspaceHistory"),
    body: `
      <div class="stack">
        <p class="muted">${t("workspaceHistoryHint")}</p>
        <div class="workspace-list">${items || `<div class="empty-state">${t("noWorkspace")}</div>`}</div>
        <div class="modal-actions">
          <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
        </div>
      </div>`,
  };
  render();
}

function openSortOptionsModal() {
  state.modal = {
    kind: "form",
    title: t("listOptions"),
    submit: "home-list-options",
    body: `
      <div class="stack">
        <label class="field">
          <span>${t("sort")}</span>
          <select name="sort">
            <option value="recent" ${state.home.sort === "recent" ? "selected" : ""}>${t("recentFirst")}</option>
            <option value="title" ${state.home.sort === "title" ? "selected" : ""}>${t("titleAsc")}</option>
          </select>
        </label>
        <label class="field">
          <span>${t("visible")}</span>
          <input type="number" min="1" max="200" name="visible" value="${state.home.visible}">
        </label>
      </div>`,
    submitLabel: t("apply"),
  };
  render();
}

function openAgentFilterModal() {
  const items = getOrderedProviders()
    .map((item) => {
      const checked = state.home.providers.includes(item.id);
      return `
        <label class="agent-pill">
          <input data-role="provider-toggle" type="checkbox" value="${escapeAttr(item.id)}" ${checked ? "checked" : ""}>
          <span>${escapeHtml(item.name)}</span>
        </label>`;
    })
    .join("");
  state.modal = {
    kind: "custom",
    title: t("terminalAgents"),
    body: `
      <div class="stack">
        <div class="agent-more-list modal-agent-list">${items || `<div class="empty-state">${t("emptySessions")}</div>`}</div>
        <div class="modal-actions">
          <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
        </div>
      </div>`,
  };
  render();
}

function openSwitchModal(provider, sessionId, workspace) {
  const defaultTarget = getDefaultSwitchTarget(provider);
  state.modal = {
    kind: "form",
    title: t("switch"),
    submit: "switch-session",
    body: `
      <div class="stack">
        <input type="hidden" name="from" value="${escapeAttr(provider)}">
        <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
        <label class="field">
          <span>${t("targetProvider")}</span>
          <select name="to">${providerOptions(provider, defaultTarget)}</select>
        </label>
        ${renderPathField("to_dir", t("targetDir"), workspace || "", t("targetWorkspaceHint"))}
      </div>
      ${renderWorkspaceDatalist()}`,
    submitLabel: t("switch"),
  };
  render();
}

function openExportModal(provider, sessionId) {
  state.modal = {
    kind: "form",
    title: t("export"),
    submit: "export-session",
    body: `
      <input type="hidden" name="provider" value="${escapeAttr(provider)}">
      <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
      <div class="stack">
        <label class="field">
          <span>${t("outputPrefix")}</span>
          <input name="output_prefix" value="${escapeAttr(sessionId)}">
        </label>
        <label class="field">
          <span>${t("format")}</span>
          <select name="format">
            <option value="json">json</option>
            <option value="md">md</option>
            <option value="html">html</option>
            <option value="morph">morph</option>
            <option value="both">both</option>
          </select>
        </label>
      </div>`,
    submitLabel: t("export"),
  };
  render();
}

function openRenameModal(provider, sessionId, title) {
  state.modal = {
    kind: "form",
    title: t("rename"),
    submit: "rename-session",
    body: `
      <input type="hidden" name="provider" value="${escapeAttr(provider)}">
      <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
      <label class="field">
        <span>${t("title")}</span>
        <input name="title" required value="${escapeAttr(title)}">
      </label>`,
    submitLabel: t("save"),
  };
  render();
}

function openDeleteModal(provider, sessionId) {
  state.modal = {
    kind: "form",
    title: t("remove"),
    submit: "delete-session",
    body: `
      <input type="hidden" name="provider" value="${escapeAttr(provider)}">
      <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
      <p>${t("deleteConfirm")}</p>`,
    submitLabel: t("remove"),
    submitClass: "danger",
  };
  render();
}

function openShareCreateModal(provider, sessionId, title) {
  const defaultTarget = getDefaultSwitchTarget(provider);
  const options = getOrderedProviders()
    .filter((item) => item.id !== provider)
    .map(
      (item) => `
      <label class="check-row">
        <input type="checkbox" name="targets" value="${escapeAttr(item.id)}"${
          item.id === defaultTarget ? " checked" : ""
        }>
        <span>${escapeHtml(item.name)}</span>
      </label>`
    )
    .join("");
  state.modal = {
    kind: "form",
    title: t("createShared"),
    submit: "create-shared",
    body: `
      <input type="hidden" name="provider" value="${escapeAttr(provider)}">
      <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
      <div class="stack">
        <label class="field">
          <span>${t("title")}</span>
          <input name="title" value="${escapeAttr(title)}">
        </label>
        ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"))}
        <div class="field">
          <span>${t("targetProviders")}</span>
          <div class="check-grid">${options}</div>
        </div>
      </div>
      ${renderWorkspaceDatalist()}`,
    submitLabel: t("create"),
  };
  render();
}

function openSharedRenameModal(groupId, title) {
  state.modal = {
    kind: "form",
    title: t("rename"),
    submit: "rename-shared",
    body: `
      <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
      <label class="field">
        <span>${t("title")}</span>
        <input name="title" value="${escapeAttr(title)}" required>
      </label>`,
    submitLabel: t("save"),
  };
  render();
}

function openSharedRemoveModal(groupId) {
  state.modal = {
    kind: "form",
    title: t("remove"),
    submit: "remove-shared",
    body: `
      <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
      <p>${t("removeSharedConfirm")}</p>
      <label class="check-row">
        <input type="checkbox" name="delete_provider_sessions" value="true">
        <span>${t("deleteProviderSessions")}</span>
      </label>`,
    submitLabel: t("remove"),
    submitClass: "danger",
  };
  render();
}

function openSharedBindModal(groupId) {
  state.modal = {
    kind: "form",
    title: t("addHolding"),
    submit: "bind-shared",
    body: `
      <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
      <div class="stack">
        <label class="field">
          <span>${t("provider")}</span>
          <select name="provider">${providerOptions()}</select>
        </label>
        <label class="field">
          <span>${t("sessionId")}</span>
          <input name="session_id" placeholder="${escapeAttr(t("emptyCreatesNewHolding"))}">
        </label>
        ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"))}
      </div>
      ${renderWorkspaceDatalist()}`,
    submitLabel: t("addHolding"),
  };
  render();
}

function openPushSyncModal(groupId, holdingId, provider, sessionId) {
  state.modal = {
    kind: "form",
    title: t("pushSyncTitle"),
    submit: "sync-from-shared",
    body: `
      <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
      <input type="hidden" name="holding_id" value="${escapeAttr(holdingId)}">
      <div class="stack">
        <p class="muted">${t("pushSyncHint")}</p>
        <div class="path-line">${escapeHtml(provider)} / ${escapeHtml(sessionId)}</div>
      </div>`,
    submitLabel: t("syncFromThis"),
    submitClass: "invert",
  };
  render();
}

function openUnbindModal(groupId, holdingId, provider, sessionId) {
  state.modal = {
    kind: "form",
    title: t("unbind"),
    submit: "unbind-shared",
    body: `
      <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
      <input type="hidden" name="holding_id" value="${escapeAttr(holdingId)}">
      <div class="stack">
        <p>${t("unbindHint")}</p>
        <div class="path-line">${escapeHtml(provider)} / ${escapeHtml(sessionId)}</div>
      </div>`,
    submitLabel: t("unbind"),
    submitClass: "danger",
  };
  render();
}

function openSyncResultModal(report) {
  const successLines = (report.success || []).map((item) => `<div class="path-line">${escapeHtml(item)}</div>`).join("");
  const errorLines = (report.errors || []).map((item) => `<div class="path-line">${escapeHtml(item)}</div>`).join("");
  state.modal = {
    kind: "custom",
    title: t("syncComplete"),
    body: `
      <div class="stack">
        <div class="status-box stack">
          <strong>${t("syncComplete")}</strong>
          <small>${escapeHtml(report.source_provider || "")}</small>
        </div>
        <div class="detail-panel stack">
          <div>
            <span class="eyebrow">${t("success")}</span>
            <div class="stack">${successLines || `<div class="muted">0</div>`}</div>
          </div>
          <div>
            <span class="eyebrow">${t("errors")}</span>
            <div class="stack">${errorLines || `<div class="muted">0</div>`}</div>
          </div>
        </div>
        <div class="modal-actions">
          <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
        </div>
      </div>`,
  };
  render();
}

function openActionResultModal({ title, summary = "", lines = [], navPath = "", navLabel = "" }) {
  state.modal = {
    kind: "custom",
    title,
    body: `
      <div class="stack">
        <div class="success-callout">
          <strong>${escapeHtml(title)}</strong>
          ${summary ? `<p>${escapeHtml(summary)}</p>` : ""}
        </div>
        ${
          lines.length
            ? `<div class="verify-block">
                <span class="block-label">${t("details")}</span>
                <div class="stack">${lines.map((line) => `<div class="path-line">${escapeHtml(line)}</div>`).join("")}</div>
              </div>`
            : ""
        }
        <div class="modal-actions">
          <button type="button" data-action="close-modal">${t("close")}</button>
          ${navPath ? `<a class="button invert" href="${escapeAttr(navPath)}" data-nav="${escapeAttr(navPath)}">${escapeHtml(navLabel || t("openDetail"))}</a>` : ""}
        </div>
      </div>`,
  };
  render();
}

function openSettingsModal(draft = null) {
  const settings = draft || state.meta.settings;
  const items = [...settings.agent_order]
    .map((providerId, index) => {
      const info = state.meta.providers.find((item) => item.id === providerId);
      const primary = settings.primary_agents.includes(providerId);
      return `
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${escapeHtml(info?.name || providerId)}</strong>
            <span>${escapeHtml(providerId)}</span>
            <input type="hidden" name="agent_order" value="${escapeAttr(providerId)}">
          </div>
          <div class="settings-agent-list">
            <label class="settings-check">
              <input type="checkbox" name="primary_agents" value="${escapeAttr(providerId)}" ${primary ? "checked" : ""}>
              <span>${t("primary")}</span>
            </label>
            <button type="button" class="ghost" data-action="shift-agent-up" data-index="${index}">${t("moveUp")}</button>
            <button type="button" class="ghost" data-action="shift-agent-down" data-index="${index}">${t("moveDown")}</button>
          </div>
        </div>`;
    })
    .join("");

  state.modal = {
    kind: "form",
    title: t("settingsTitle"),
    submit: "save-settings",
    body: `
      <div class="settings-list">
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${t("language")}</strong>
            <span>${t("settingsLanguageHint")}</span>
          </div>
          <select name="language">
            <option value="zh" ${settings.language === "zh" ? "selected" : ""}>中文</option>
            <option value="en" ${settings.language === "en" ? "selected" : ""}>English</option>
          </select>
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${t("sessionsPerProvider")}</strong>
            <span>${t("settingsSessionsHint")}</span>
          </div>
          <input type="number" min="1" max="200" name="sessions_per_provider" value="${settings.sessions_per_provider}">
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <strong>OpenCode subagents</strong>
            <span>${t("settingsSubagentsHint")}</span>
          </div>
          <label class="settings-check">
            <input type="checkbox" name="show_opencode_subagents" value="true" ${settings.show_opencode_subagents ? "checked" : ""}>
            <span>${t("done")}</span>
          </label>
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${t("homeButtons")}</strong>
            <span>${t("settingsHomeButtonsHint")}</span>
          </div>
          <div class="settings-agent-list">
            <label class="settings-check">
              <input type="checkbox" name="home_button_view" value="true" ${settings.home_buttons.view ? "checked" : ""}>
              <span>${t("showView")}</span>
            </label>
            <label class="settings-check">
              <input type="checkbox" name="home_button_switch" value="true" ${settings.home_buttons.switch ? "checked" : ""}>
              <span>${t("showSwitch")}</span>
            </label>
            <label class="settings-check">
              <input type="checkbox" name="home_button_export" value="true" ${settings.home_buttons.export ? "checked" : ""}>
              <span>${t("showExport")}</span>
            </label>
            <label class="settings-check">
              <input type="checkbox" name="home_button_share" value="true" ${settings.home_buttons.share ? "checked" : ""}>
              <span>${t("showShare")}</span>
            </label>
            <label class="settings-check">
              <input type="checkbox" name="home_button_delete" value="true" ${settings.home_buttons.delete ? "checked" : ""}>
              <span>${t("showDelete")}</span>
            </label>
          </div>
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${t("providers")}</strong>
            <span>${t("settingsProvidersHint")}</span>
          </div>
          <div class="settings-provider-list">${items}</div>
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <strong>${t("version")}</strong>
            <span>v${escapeHtml(state.meta?.version || "")}</span>
          </div>
          <a class="button" href="https://www.npmjs.com/package/memorph" target="_blank" rel="noopener noreferrer">${t("checkUpdate")}</a>
        </div>
      </div>`,
    submitLabel: t("save"),
  };
  render();
}

function openManagerModal(draft = null, preview = null, report = null) {
  const managerDraft = draft || defaultManagerDraft();
  const providerChecks = getOrderedProviders()
    .map(
      (item) => `
      <label class="check-row">
        <input type="checkbox" name="providers" value="${escapeAttr(item.id)}"${
          managerDraft.providers.includes(item.id) ? " checked" : ""
        }>
        <span>${escapeHtml(item.name)}</span>
      </label>`
    )
    .join("");

  const previewSection = preview
    ? renderManagerPreview(preview, report)
    : `<div class="empty-state"><p>${t("managerTitle")}</p></div>`;

  state.modal = {
    kind: "custom",
    title: t("managerTitle"),
    body: `
      <div class="manager-layout">
        <form class="stack" data-submit="preview-manager">
          ${renderPathField("workspace", t("workspace"), managerDraft.workspace, t("managerWorkspaceHint"))}
          <label class="field">
            <span>${t("olderThanDays")}</span>
            <input type="number" min="0" name="older_than_days" placeholder="30" value="${escapeAttr(
              managerDraft.older_than_days
            )}">
          </label>
          <label class="field">
            <span>${t("largerThanMb")}</span>
            <input type="number" min="0" name="larger_than_mb" placeholder="1" value="${escapeAttr(
              managerDraft.larger_than_mb
            )}">
          </label>
          <div class="stack">
            <span class="eyebrow">${t("providers")}</span>
            <div class="check-grid">${providerChecks}</div>
          </div>
          <button class="invert" type="submit">${t("preview")}</button>
        </form>
        ${renderWorkspaceDatalist()}
        <div class="stack">${previewSection}</div>
      </div>`,
  };
  render();
}

function renderManagerPreview(preview, report = null) {
  const rows = preview.items
    .map((item) => {
      const encoded = escapeAttr(encodeURIComponent(JSON.stringify(item)));
      return `
        <label class="manager-row">
          <div class="manager-row-head">
            <label>
              <input type="checkbox" name="manager_item" value="${encoded}">
              <span>
                <strong>${escapeHtml(item.title || item.session_id)}</strong>
                <div class="manager-meta">
                  <span>${escapeHtml(item.provider_name)}</span>
                  <span>${escapeHtml(formatBytes(item.size_bytes))}</span>
                  <span>${escapeHtml(formatDate(item.last_active_at))}</span>
                </div>
              </span>
            </label>
          </div>
          <div class="path-line">${escapeHtml(item.project_dir || item.source_path || "")}</div>
        </label>`;
    })
    .join("");
  const reportSection = report
    ? `
      <div class="success-callout">
        <strong>${escapeHtml(report.title)}</strong>
        <p>${escapeHtml(report.summary)}</p>
      </div>
      ${
        report.lines.length
          ? `<div class="verify-block">
              <span class="block-label">${escapeHtml(report.toggleLabel)}</span>
              <div class="stack">${report.lines.map((line) => `<div class="path-line">${escapeHtml(line)}</div>`).join("")}</div>
            </div>`
          : ""
      }
      `
    : "";
  return `
    ${reportSection}
    <div class="section-heading manager-section-head">
      <div>
        <strong>${t("managerPreview")}</strong>
        <small>${t("managerSummary").replace("{count}", preview.total_count).replace("{size}", formatBytes(preview.total_size_bytes))}</small>
      </div>
      <label class="check-row">
        <input type="checkbox" data-role="select-all-manager">
        <span>${t("selectAll")}</span>
      </label>
    </div>
    <div class="manager-list">${rows || `<div class="empty-state">${t("emptySessions")}</div>`}</div>
    <div class="modal-actions">
      <form data-submit="clean-manager"><button class="danger" type="submit">${t("cleanSelected")}</button></form>
      <form data-submit="backup-manager" class="actions-inline">
        <input name="output_dir" placeholder="${escapeAttr(t("backupDirPlaceholder"))}" value="${escapeAttr(
          preview.output_dir || ""
        )}">
        <button type="submit">${t("backupSelected")}</button>
      </form>
    </div>`;
}

async function runImport(formData) {
  const result = await api("/api/v1/import", {
    method: "POST",
    body: {
      provider: String(formData.get("provider")),
      file_or_id: String(formData.get("file_or_id")),
      to_dir: emptyToNull(formData.get("to_dir")),
    },
  });
  await loadHome();
  closeModal();
  openActionResultModal({
    title: t("imported"),
    summary: result.provider_name,
    lines: [
      `${t("target")}: ${result.provider_name}`,
      `${t("sessionId")}: ${result.new_session_id}`,
      ...(result.resume_command ? [`${t("resumeCommand")}: ${result.resume_command}`] : []),
    ],
    navPath: `/sessions/${encodeURIComponent(String(formData.get("provider")))}/${encodeURIComponent(result.new_session_id)}`,
  });
}

async function runSwitch(formData) {
  const result = await api("/api/v1/switch", {
    method: "POST",
    body: {
      from: String(formData.get("from")),
      to: String(formData.get("to")),
      session_id: emptyToNull(formData.get("session_id")),
      to_dir: emptyToNull(formData.get("to_dir")),
    },
  });
  await loadHome();
  closeModal();
  openActionResultModal({
    title: t("switched"),
    summary: `${result.from_name} → ${result.to_name}`,
    lines: [
      `${t("source")}: ${result.from_name} / ${result.source_session_id}`,
      `${t("target")}: ${result.to_name} / ${result.target_session_id}`,
      ...(result.resume_command ? [`${t("resumeCommand")}: ${result.resume_command}`] : []),
    ],
    navPath: `/sessions/${encodeURIComponent(String(formData.get("to")))}/${encodeURIComponent(result.target_session_id)}`,
  });
}

async function runExport(formData) {
  const result = await api("/api/v1/export", {
    method: "POST",
    body: {
      provider: String(formData.get("provider")),
      session_id: String(formData.get("session_id")),
      output_prefix: emptyToNull(formData.get("output_prefix")),
      format: String(formData.get("format")),
    },
  });
  closeModal();
  openActionResultModal({
    title: t("exported"),
    lines: result.files,
  });
}

async function runRename(formData) {
  const provider = String(formData.get("provider"));
  const sessionId = String(formData.get("session_id"));
  const title = String(formData.get("title"));
  await api(`/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`, {
    method: "PATCH",
    body: { title },
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("rename"),
    summary: t("saved"),
    lines: [
      `${t("provider")}: ${provider}`,
      `${t("sessionId")}: ${sessionId}`,
      `${t("title")}: ${title}`,
    ],
    navPath: `/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`,
  });
}

async function runDelete(formData) {
  const provider = String(formData.get("provider"));
  const sessionId = String(formData.get("session_id"));
  await api(`/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`, {
    method: "DELETE",
  });
  if (state.route.name === "session") {
    replacePath("/");
    state.session = null;
  }
  await loadHome();
  closeModal();
  openActionResultModal({
    title: t("deleted"),
    lines: [
      `${t("provider")}: ${provider}`,
      `${t("sessionId")}: ${sessionId}`,
    ],
    navPath: "/",
    navLabel: t("openHome"),
  });
}

async function runShareCreate(formData) {
  const targets = formData.getAll("targets").map(String);
  const result = await api("/api/v1/share", {
    method: "POST",
    body: {
      provider: String(formData.get("provider")),
      session_id: String(formData.get("session_id")),
      targets,
      to_dir: emptyToNull(formData.get("to_dir")),
      title: emptyToNull(formData.get("title")),
    },
  });
  await loadHome();
  closeModal();
  openActionResultModal({
    title: t("sharedCreated"),
    summary: result.id,
    lines: [
      `${t("sessionId")}: ${result.id}`,
      `${t("holdings")}: ${result.holdings.length}`,
    ],
    navPath: `/shared/${encodeURIComponent(result.id)}`,
    navLabel: t("openDetail"),
  });
}

async function runSharedRename(formData) {
  const groupId = String(formData.get("group_id"));
  const title = String(formData.get("title"));
  await api(`/api/v1/share/${encodeURIComponent(groupId)}`, {
    method: "PATCH",
    body: { title },
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("rename"),
    summary: t("sharedTitle"),
    lines: [
      `${t("sessionId")}: ${groupId}`,
      `${t("title")}: ${title}`,
    ],
    navPath: `/shared/${encodeURIComponent(groupId)}`,
    navLabel: t("openDetail"),
  });
}

async function runSharedRemove(formData) {
  const groupId = String(formData.get("group_id"));
  const removeUrl = new URL(`/api/v1/share/${encodeURIComponent(groupId)}`, window.location.origin);
  removeUrl.searchParams.set("delete_provider_sessions", formData.get("delete_provider_sessions") ? "true" : "false");
  await api(removeUrl.pathname + removeUrl.search, { method: "DELETE" });
  state.home.sharedGroups = (state.home.sharedGroups || []).filter((group) => group.id !== groupId);
  if (state.route.name === "shared-detail" && state.route.groupId === groupId) {
    replacePath("/shared");
    state.sharedDetail = null;
  }
  closeModal();
  openActionResultModal({
    title: t("deleted"),
    lines: [
      `${t("sessionId")}: ${groupId}`,
      `${t("sharedTitle")}: ${groupId}`,
    ],
    navPath: "/shared",
    navLabel: t("sharedGroups"),
  });
}

async function runSharedBind(formData) {
  const result = await api("/api/v1/share/bind", {
    method: "POST",
    body: {
      group_id: String(formData.get("group_id")),
      provider: String(formData.get("provider")),
      session_id: emptyToNull(formData.get("session_id")),
      to_dir: emptyToNull(formData.get("to_dir")),
    },
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("addHolding"),
    summary: t("saved"),
    lines: [
      `${t("target")}: ${result.provider}`,
      `${t("sessionId")}: ${result.session_id}`,
      ...(result.target_dir ? [`${t("workspace")}: ${result.target_dir}`] : []),
    ],
    navPath: `/sessions/${encodeURIComponent(result.provider)}/${encodeURIComponent(result.session_id)}`,
  });
}

async function runSharedSync(groupId, sourceHoldingId = null) {
  const result = await api("/api/v1/share/sync", {
    method: "POST",
    body: {
      group_id: groupId,
      source_holding_id: sourceHoldingId,
    },
  });
  await loadRoute();
  closeModal();
  openSyncResultModal(result);
}

async function runUnbind(groupId, holdingId) {
  await api(`/api/v1/share/holdings/${encodeURIComponent(groupId)}/${encodeURIComponent(holdingId)}`, {
    method: "DELETE",
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("unbind"),
    summary: t("deleted"),
    lines: [
      `${t("sessionId")}: ${holdingId}`,
      `${t("sharedTitle")}: ${groupId}`,
    ],
    navPath: `/shared/${encodeURIComponent(groupId)}`,
    navLabel: t("openDetail"),
  });
}

async function runSaveSettings(formData) {
  const body = {
    sessions_per_provider: Number(formData.get("sessions_per_provider")),
    language: String(formData.get("language")),
    show_opencode_subagents: formData.get("show_opencode_subagents") === "true",
    home_buttons: {
      view: formData.get("home_button_view") === "true",
      switch: formData.get("home_button_switch") === "true",
      export: formData.get("home_button_export") === "true",
      share: formData.get("home_button_share") === "true",
      delete: formData.get("home_button_delete") === "true",
    },
    agent_order: formData.getAll("agent_order").map(String),
    primary_agents: formData.getAll("primary_agents").map(String),
  };
  await saveSettings(body);
  closeModal();
}

async function saveSettings(body) {
  state.meta.settings = await api("/api/v1/settings", {
    method: "PUT",
    body,
  });
  setDocumentLanguage();
  state.home.visible = state.meta.settings.sessions_per_provider;
  toast(t("saved"), t("settingsTitle"));
  render();
}

async function runManagerPreview(formData) {
  const draft = managerDraftFromFormData(formData);
  const preview = await api("/api/v1/manager/preview", {
    method: "POST",
    body: managerPreviewBody(draft),
  });
  preview.output_dir = "";
  openManagerModal(draft, preview);
}

async function runManagerClean() {
  const draft = readManagerDraft();
  const items = selectedManagerItems();
  if (!items.length) throw new Error(t("noSelection"));
  const result = await api("/api/v1/manager/clean", {
    method: "POST",
    body: { items },
  });
  toast(t("done"), `${result.success} ${t("managerCleaned")}`);
  const preview = await api("/api/v1/manager/preview", {
    method: "POST",
    body: managerPreviewBody(draft),
  });
  preview.output_dir = "";
  const report = {
    title: t("cleanSelected"),
    summary: `${result.success} ${t("managerCleaned")}, ${result.failed} ${t("managerFailed")}, ${formatBytes(
      result.freed_bytes
    )} ${t("managerFreed")}`,
    toggleLabel: t("errors"),
    lines: result.errors || [],
  };
  openManagerModal(draft, preview, report);
}

async function runManagerBackup(formData) {
  const draft = readManagerDraft();
  const items = selectedManagerItems();
  if (!items.length) throw new Error(t("noSelection"));
  const outputDir = emptyToNull(formData.get("output_dir")) || t("backupDirPlaceholder");
  const result = await api("/api/v1/manager/backup", {
    method: "POST",
    body: {
      items,
      output_dir: outputDir,
    },
  });
  toast(t("backup"), result.files.join("\n"));
  const preview = await api("/api/v1/manager/preview", {
    method: "POST",
    body: managerPreviewBody(draft),
  });
  preview.output_dir = outputDir;
  const report = {
    title: t("backupSelected"),
    summary: `${result.success} ${t("managerExported")}, ${result.failed} ${t("managerFailed")}`,
    toggleLabel: result.errors?.length ? t("filesAndErrors") : t("files"),
    lines: [...(result.files || []), ...(result.errors || [])],
  };
  openManagerModal(draft, preview, report);
}

function selectedManagerItems() {
  return [...document.querySelectorAll('input[name="manager_item"]:checked')].map((el) =>
    JSON.parse(decodeURIComponent(el.value))
  );
}

function defaultManagerDraft() {
  return {
    workspace: state.home.workspace || "",
    older_than_days: "",
    larger_than_mb: "",
    providers: [],
  };
}

function managerDraftFromFormData(formData) {
  return {
    workspace: String(formData.get("workspace") || ""),
    older_than_days: String(formData.get("older_than_days") || ""),
    larger_than_mb: String(formData.get("larger_than_mb") || ""),
    providers: formData.getAll("providers").map(String),
  };
}

function readManagerDraft() {
  const form = document.querySelector('form[data-submit="preview-manager"]');
  if (!form) return defaultManagerDraft();
  return managerDraftFromFormData(new FormData(form));
}

function managerPreviewBody(draft) {
  return {
    workspace: emptyToNull(draft.workspace),
    older_than_days: numberOrNull(draft.older_than_days),
    larger_than_mb: numberOrNull(draft.larger_than_mb),
    providers: draft.providers,
  };
}

function shiftAgent(direction, index) {
  const draft = readSettingsDraft();
  const order = [...draft.agent_order];
  const target = direction === "up" ? index - 1 : index + 1;
  if (target < 0 || target >= order.length) return;
  const next = [...order];
  [next[index], next[target]] = [next[target], next[index]];
  draft.agent_order = next;
  openSettingsModal(draft);
}

function readSettingsDraft() {
  const form = document.querySelector('form[data-submit="save-settings"]');
  if (!form) {
    return structuredClone(state.meta.settings);
  }

  const formData = new FormData(form);
  return {
    sessions_per_provider: Number(formData.get("sessions_per_provider") || state.meta.settings.sessions_per_provider),
    language: String(formData.get("language") || state.meta.settings.language),
    show_opencode_subagents: formData.get("show_opencode_subagents") === "true",
    home_buttons: {
      view: formData.get("home_button_view") === "true",
      switch: formData.get("home_button_switch") === "true",
      export: formData.get("home_button_export") === "true",
      share: formData.get("home_button_share") === "true",
      delete: formData.get("home_button_delete") === "true",
    },
    agent_order: formData.getAll("agent_order").map(String),
    primary_agents: formData.getAll("primary_agents").map(String),
  };
}

async function api(path, options = {}) {
  const request = {
    method: options.method || "GET",
    headers: {
      Accept: "application/json",
    },
  };
  if (options.body !== undefined) {
    request.headers["Content-Type"] = "application/json";
    request.body = JSON.stringify(options.body);
  }

  const response = await fetch(path, request);
  const raw = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(raw?.error || `HTTP ${response.status}`);
  }
  if (raw?.ok) return raw.data;
  return raw;
}

function render() {
  if (!state.meta) {
    appEl.innerHTML = `
      <div class="app-shell">
        <nav class="topbar">
          <div class="brand">memorph</div>
        </nav>
        <main class="app-main">${renderLoading()}</main>
      </div>`;
    modalRoot.innerHTML = "";
    return;
  }
  appEl.innerHTML = `
    <div class="app-shell">
      <nav class="topbar">
        <div class="brand-cluster">
          <a class="brand" href="/" data-nav="/">memorph</a>
        </div>
        <div class="top-actions">
          <a class="button" href="/shared" data-nav="/shared">${t("shared")}</a>
          <button type="button" data-action="open-workspace-switch">${t("switchWorkspace")}</button>
          <button type="button" data-action="open-import">${t("import")}</button>
          <button type="button" data-action="open-manager">${t("manage")}</button>
          <button type="button" data-action="open-settings">${t("settings")}</button>
          <a class="icon-button" href="https://github.com/ip2a/memorph" target="_blank" rel="noopener noreferrer" aria-label="GitHub repository" title="GitHub">
            ${githubIcon()}
          </a>
        </div>
      </nav>
      <main class="app-main">${renderPage()}</main>
    </div>
    ${renderLoading()}
    ${renderToasts()}
  `;
  renderModal();
  bindLocalButtons();
}

function bindLocalButtons() {
  document.querySelectorAll('[data-action="shift-agent-up"]').forEach((button) => {
    button.addEventListener("click", () => shiftAgent("up", Number(button.dataset.index)));
  });
  document.querySelectorAll('[data-action="shift-agent-down"]').forEach((button) => {
    button.addEventListener("click", () => shiftAgent("down", Number(button.dataset.index)));
  });
  scheduleHomeProviderLayout();
}

function renderPage() {
  switch (state.route.name) {
    case "home":
      return renderHome();
    case "session":
      return `<div class="page-scroll">${renderSessionDetail()}</div>`;
    case "shared-list":
      return `<div class="page-scroll">${renderSharedList()}</div>`;
    case "shared-detail":
      return `<div class="page-scroll">${renderSharedDetail()}</div>`;
    default:
      return `<div class="page-scroll"><div class="empty-state">${t("notFound")}</div></div>`;
  }
}

function renderHome() {
  const filteredGroups = sortSessionGroupsByDisplay(filterAndSortGroups(state.home.groups));
  const totalSessions = filteredGroups.reduce((sum, group) => sum + (group.total_sessions || group.sessions.length), 0);
  const shownSessions = filteredGroups.reduce((sum, group) => sum + group.sessions.length, 0);
  return `
    <div class="page-home">
      <section class="home-hero">
        <div class="ascii-banner"><pre>${escapeHtml(ASCII)}</pre></div>
        <div class="workspace-hero">
          <p class="eyebrow">${t("workspace")}</p>
          <h1>${escapeHtml(workspaceName(state.home.workspace) || "memorph")}</h1>
          <button type="button" class="workspace-path" data-action="open-workspace-switch" title="${escapeAttr(state.home.workspace || "")}">
            ${escapeHtml(state.home.workspace || "—")}
          </button>
          <div class="meta-line">
            <span>${t("sessionsStat")}=${totalSessions}</span>
            <span>${t("terminalAgents")}=${filteredGroups.length}</span>
            <span>${t("shown")}=${shownSessions}</span>
          </div>
        </div>
      </section>
      <section class="section-panel session-results">
        <div class="section-heading home-list-head">
          <div>
            <strong>${t("recentSessions")}</strong>
            <small>${t("filters")}</small>
          </div>
          <div class="home-list-controls">
            ${renderProviderPicker()}
            <form class="session-search" id="home-search-form" data-submit="home-search">
              <input data-role="home-search" name="search" value="${escapeAttr(state.home.search)}" placeholder="${escapeAttr(
                t("searchPlaceholder")
              )}">
            </form>
            <button type="button" data-action="open-sort-options">${t("sort")}</button>
            <button type="submit" form="home-search-form">${t("filters")}</button>
          </div>
        </div>
        ${renderHomeGroups(filteredGroups, totalSessions, shownSessions)}
      </section>
    </div>`;
}

function renderWorkspacePicker() {
  return `
    <label class="field field-wide">
      <span>${t("workspacePath")}</span>
      <div class="workspace-combo">
        <input name="workspace" list="known-workspaces" value="${escapeAttr(state.home.workspace || "")}" placeholder="${escapeAttr(
          state.meta?.workspaces?.[0]?.path || ""
        )}">
        <button type="button" class="ghost" data-action="browse-folder" data-target-field="workspace">${t("browse")}</button>
        <button type="button" class="ghost" data-action="open-workspace-history">${t("history")}</button>
        <button type="submit" class="invert">${t("go")}</button>
      </div>
      <small class="muted">${t("workspaceFieldHint")}</small>
    </label>`;
}

function renderProviderPicker() {
  const primary = getVisibleToolbarProviders();
  const primaryMarkup = primary
    .map((item) => {
      const checked = state.home.providers.includes(item.id);
      return `
        <label class="agent-pill">
          <input data-role="provider-toggle" type="checkbox" value="${escapeAttr(item.id)}" ${checked ? "checked" : ""}>
          <span>${escapeHtml(item.name)}</span>
        </label>`;
    })
    .join("");

  return `
    <div class="agent-picker-shell home-provider-strip">
      <div class="agent-picker">${primaryMarkup}</div>
      <button type="button" class="agent-more-button" data-action="open-agent-filter">${t("more")}</button>
    </div>`;
}

function renderHomeGroups(groups, totalSessions, shownSessions) {
  if (!groups.length) {
    return `<div class="empty-state">${t("emptySessions")}</div>`;
  }
  return `
    ${groups
    .map(
      (group) => `
      <details class="provider-section" open>
        <summary>
          <span>${escapeHtml(group.provider_name)}</span>
          <span>${group.shown_sessions || group.sessions.length}/${group.total_sessions || group.sessions.length}</span>
        </summary>
        <div class="session-list">
          ${group.sessions.map((item) => renderSessionRow(item)).join("")}
        </div>
      </details>`
    )
    .join("")}`;
}

function renderSessionRow(item) {
  const sharedRef = findSharedRef(item.provider_id, item.session_id);
  const buttons = state.meta.settings.home_buttons;
  const shareAction = sharedRef
    ? `<a class="button" href="/shared/${sharedRef}" data-nav="/shared/${sharedRef}">${t("openShared")}</a>`
    : `<button type="button" data-action="open-share-create" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-title="${escapeAttr(item.title || "")}">${t("share")}</button>`;
  return `
    <article class="session-row">
      <div class="session-row-main">
        <div class="session-info">
          <div class="session-title-line">
            <span class="session-title">${escapeHtml(item.title || item.session_id)}</span>
            <span class="session-workspace">${escapeHtml(item.project_dir || "—")}</span>
          </div>
          <div class="session-meta-bar">
            <span class="session-id-pill">${escapeHtml(item.session_id)}</span>
            ${sharedRef ? `<a class="shared-badge" href="/shared/${sharedRef}" data-nav="/shared/${sharedRef}">${t("activeShared")}</a>` : ""}
            <span class="meta-dot">·</span>
            <span class="meta-item" title="${escapeAttr(t("lastActiveAt"))}">${escapeHtml(formatDate(item.last_active_at))}</span>
            <span class="meta-dot">·</span>
            <span class="meta-item" title="${escapeAttr(t("messageCount"))}">${escapeHtml(String(item.message_count ?? "—"))}</span>
            <span class="meta-dot">·</span>
            <span class="meta-item" title="${escapeAttr(t("size"))}">${escapeHtml(formatBytes(item.size_bytes))}</span>
          </div>
        </div>
        <div class="row-actions">
        ${buttons.view ? `<a class="button" href="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}" data-nav="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}">${t("view")}</a>` : ""}
        ${buttons.switch ? `<button type="button" data-action="open-switch" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-workspace="${escapeAttr(item.project_dir || state.home.workspace)}">${t("switch")}</button>` : ""}
        ${buttons.export ? `<button type="button" data-action="open-export" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}">${t("export")}</button>` : ""}
        ${buttons.share ? shareAction : ""}
        <button type="button" data-action="open-rename" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-title="${escapeAttr(item.title || "")}">${t("rename")}</button>
        ${buttons.delete ? `<button type="button" class="danger" data-action="open-delete" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}">${t("remove")}</button>` : ""}
        </div>
      </div>
    </article>`;
}

function renderSessionDetail() {
  if (!state.session) return `<div class="empty-state">${t("loading")}</div>`;
  const detail = state.session;
  if (!detail.view) return `<div class="empty-state">${t("loading")}</div>`;
  return renderSessionDetailView(detail, detail.view);
}

function renderSessionDetailView(detail, view) {
  const sharedRef = findSharedRef(state.route.provider, state.route.sessionId);
  const workspace = view.workspace_dir || state.home.workspace || "";
  return `
    <section class="session-header">
      <div>
        <p class="eyebrow">${escapeHtml(view.provider_name || view.provider_id)}</p>
        <h1>${escapeHtml(view.title || view.session_id || state.route.sessionId)}</h1>
        <div class="meta-line">
          <span>id=<code>${escapeHtml(view.session_id || state.route.sessionId)}</code></span>
          <span>${t("messageCount")}=${view.message_count}</span>
          ${workspace ? `<span>${t("workspace")}=<code>${escapeHtml(workspace)}</code></span>` : ""}
          ${sharedRef ? `<a class="shared-badge" href="/shared/${sharedRef}" data-nav="/shared/${sharedRef}">${t("activeShared")}</a>` : ""}
        </div>
      </div>
      <div class="session-actions">
        <a class="button" href="/" data-nav="/">${t("back")}</a>
        ${sharedRef ? `<a class="button" href="/shared/${sharedRef}" data-nav="/shared/${sharedRef}">${t("openShared")}</a>` : ""}
        <button type="button" data-action="open-share-create" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-title="${escapeAttr(view.title || "")}">${t("share")}</button>
        <button type="button" data-action="open-switch" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-workspace="${escapeAttr(workspace)}">${t("switch")}</button>
        <button type="button" data-action="open-export" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}">${t("export")}</button>
        <button type="button" data-action="open-rename" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}" data-title="${escapeAttr(view.title || "")}">${t("rename")}</button>
        <button type="button" class="danger" data-action="open-delete" data-provider="${escapeAttr(state.route.provider)}" data-session-id="${escapeAttr(state.route.sessionId)}">${t("remove")}</button>
      </div>
    </section>
    <div class="detail-layout">
      <section>
        <div class="msg-list">
          ${view.events.length ? view.events.map(renderDetailEvent).join("") : `<div class="empty-state">${t("noMessages")}</div>`}
        </div>
      </section>
      <aside class="detail-panel stack">
        <div>
          <h3>${t("details")}</h3>
          <p class="muted">${t("overview")}</p>
        </div>
        ${renderMetaLine(t("provider"), view.provider_name || view.provider_id)}
        ${renderMetaLine(t("sessionId"), view.session_id || state.route.sessionId)}
        ${renderMetaLine(t("messageCount"), String(view.message_count))}
        ${renderMetaLine(t("projectDir"), view.workspace_dir)}
        ${renderMetaLine(t("createdAt"), view.created_at)}
        ${renderMetaLine(t("lastActiveAt"), formatDate(view.last_active_at))}
        ${renderMetaLine(t("sourcePath"), view.source_path)}
        ${renderMetaLine(t("resumeCommand"), view.resume_command || "")}
      </aside>
    </div>`;
}

function renderDetailEvent(event) {
  const blocks = (event.blocks || []).map(renderDetailBlock).join("");
  const role = (event.role || "unknown").replaceAll("_", " ");
  const kind = (event.kind || "unknown").replaceAll("_", " ");
  return `
    <article class="msg-item">
      <header class="msg-header">
        <span class="msg-role">${escapeHtml(role)}</span>
        <span>${escapeHtml(kind)}</span>
        <span>${escapeHtml(formatDate(event.timestamp))}</span>
      </header>
      <div class="msg-body">${blocks || `<p class="muted">${t("noDetails")}</p>`}</div>
    </article>`;
}

function renderDetailBlock(block) {
  switch (block.type) {
    case "text":
      return `<div>${markdown(block.text || "")}</div>`;
    case "thinking":
      return `<details class="thinking-block"><summary class="block-label">${t("thinking")}</summary><p>${escapeHtml(block.text || "")}</p></details>`;
    case "tool_call":
      return `<details class="tool-block"><summary class="block-label">${escapeHtml(
        `${t("toolUse")}: ${block.name || ""}`.replace(/:\s$/, "")
      )}</summary><pre><code>${escapeHtml(
        JSON.stringify(
          { tool_call_id: block.tool_call_id, name: block.name, input: block.input },
          null,
          2
        )
      )}</code></pre></details>`;
    case "tool_result":
      return `<details class="tool-block"><summary class="block-label">${t("toolResult")}</summary><pre><code>${escapeHtml(
        block.content || ""
      )}</code></pre></details>`;
    case "patch":
      return `<details class="tool-block"><summary class="block-label">Patch</summary><pre><code>${escapeHtml(
        block.diff_text ||
          JSON.stringify(
            {
              summary: block.summary,
              files: block.files,
              hash: block.hash,
            },
            null,
            2
          )
      )}</code></pre></details>`;
    case "command":
      return `<details class="tool-block"><summary class="block-label">Command</summary><pre><code>${escapeHtml(
        JSON.stringify(
          {
            command: block.command,
            argv: block.argv,
            cwd: block.cwd,
          },
          null,
          2
        )
      )}</code></pre></details>`;
    case "command_result":
      return `<details class="tool-block"><summary class="block-label">Command Result</summary><pre><code>${escapeHtml(
        JSON.stringify(
          {
            command: block.command,
            exit_code: block.exit_code,
            stdout: block.stdout,
            stderr: block.stderr,
          },
          null,
          2
        )
      )}</code></pre></details>`;
    case "file":
      return `<div class="tool-block"><span class="block-label">${t("file")}</span><code>${escapeHtml(block.path || "")}</code>${
        block.content ? `<pre><code>${escapeHtml(block.content)}</code></pre>` : ""
      }</div>`;
    case "image":
      return `<div class="tool-block"><span class="block-label">${t("image")}</span><code>${escapeHtml(
        block.path || block.mime_type || ""
      )}</code></div>`;
    case "provider_payload":
      return `<details class="tool-block"><summary class="block-label">${escapeHtml(
        block.kind || "payload"
      )}</summary><pre><code>${escapeHtml(
        JSON.stringify(block.payload ?? {}, null, 2)
      )}</code></pre></details>`;
    case "unknown":
      return `<details class="tool-block"><summary class="block-label">${t("details")}</summary><pre><code>${escapeHtml(
        JSON.stringify(block.raw ?? block, null, 2)
      )}</code></pre></details>`;
    default:
      return `<pre>${escapeHtml(JSON.stringify(block, null, 2))}</pre>`;
  }
}

function renderMessage(message) {
  const blocks = (message.content || []).map(renderContentBlock).join("");
  return `
    <article class="msg-item">
      <header class="msg-header">
        <span class="msg-role">${escapeHtml(message.role)}</span>
        <span>${escapeHtml(formatDate(message.timestamp))}</span>
      </header>
      <div class="msg-body">${blocks || `<p class="muted">${t("noDetails")}</p>`}</div>
    </article>`;
}

function renderContentBlock(block) {
  switch (block.type) {
    case "text":
      return `<div>${markdown(block.text || "")}</div>`;
    case "thinking":
      return `<details class="thinking-block"><summary class="block-label">${t("thinking")}</summary><p>${escapeHtml(block.thinking || "")}</p></details>`;
    case "tool_use":
      return `<details class="tool-block"><summary class="block-label">${escapeHtml(
        `${t("toolUse")}: ${block.name || ""}`.replace(/:\s$/, "")
      )}</summary><pre><code>${escapeHtml(
        JSON.stringify({ id: block.id, name: block.name, input: block.input }, null, 2)
      )}</code></pre></details>`;
    case "tool_result":
      return `<details class="tool-block"><summary class="block-label">${t("toolResult")}</summary><pre><code>${escapeHtml(
        block.content || ""
      )}</code></pre></details>`;
    case "file":
      return `<div class="tool-block"><span class="block-label">${t("file")}</span><code>${escapeHtml(block.path || "")}</code>${
        block.content ? `<pre><code>${escapeHtml(block.content)}</code></pre>` : ""
      }</div>`;
    case "image":
      return `<div class="tool-block"><span class="block-label">${t("image")}</span><code>${escapeHtml(
        block.mime_type || ""
      )}</code></div>`;
    default:
      return `<pre>${escapeHtml(JSON.stringify(block, null, 2))}</pre>`;
  }
}

function renderSharedList() {
  const groups = state.home.sharedGroups || [];
  const totalHoldings = groups.reduce((sum, group) => sum + (group.holdings?.length || 0), 0);
  return `
    <section class="session-header">
      <div>
        <p class="eyebrow">${t("sharedTitle")}</p>
        <h1>${t("sharedTitle")}</h1>
        <div class="meta-line">
          <span>${t("sessionsStat")}=${groups.length}</span>
          <span>${t("holdings")}=${totalHoldings}</span>
        </div>
      </div>
      <div class="session-actions">
        <a class="button" href="/" data-nav="/">${t("back")}</a>
      </div>
    </section>
    ${groups.length ? `<div class="shared-list">${groups.map(renderSharedRow).join("")}</div>` : `<div class="empty-state">${t("noSharedGroups")}</div>`}`;
}

function renderSharedRow(group) {
  const sourceProvider = getOrderedProviders().find((item) => item.id === group.source_provider);
  const bindingStrip = (group.holdings || [])
    .map((holding) => {
      const provider = getOrderedProviders().find((item) => item.id === holding.provider);
      return `<span class="status-pill">${escapeHtml(provider?.name || holding.provider)}:${escapeHtml(shortId(holding.session_id))}</span>`;
    })
    .join("");
  return `
    <article class="shared-row">
      <span class="session-id">${escapeHtml(group.id)}</span>
      <span class="session-title">${escapeHtml(group.title || group.id)}</span>
      <div class="session-meta">
        <span>${t("holdings")}=${group.holdings.length}</span>
        <span>${t("updatedAt")}=${escapeHtml(formatDate(group.updated_at))}</span>
      </div>
      <div class="binding-strip">${bindingStrip || `<span class="status-pill">${escapeHtml(sourceProvider?.name || group.source_provider || "—")}</span>`}</div>
      <div class="row-actions">
        <a class="button" href="/shared/${encodeURIComponent(group.id)}" data-nav="/shared/${encodeURIComponent(group.id)}">${t("view")}</a>
        <button type="button" data-action="run-sync-latest" data-group-id="${escapeAttr(group.id)}">${t("syncLatest")}</button>
        <button type="button" data-action="open-shared-rename" data-group-id="${escapeAttr(group.id)}" data-title="${escapeAttr(group.title || "")}">${t("rename")}</button>
        <button type="button" class="danger" data-action="open-shared-remove" data-group-id="${escapeAttr(group.id)}">${t("remove")}</button>
      </div>
    </article>`;
}

function renderSharedDetail() {
  if (!state.sharedDetail) return `<div class="empty-state">${t("loading")}</div>`;
  const group = state.sharedDetail;
  const sourceProvider = getOrderedProviders().find((item) => item.id === group.source_provider);
  return `
    <section class="session-header">
      <div>
        <p class="eyebrow">${t("details")}</p>
        <h1>${escapeHtml(group.title || group.id)}</h1>
        <div class="meta-line">
          <span>id=<code>${escapeHtml(group.id)}</code></span>
          <span>${t("holdings")}=${group.holdings.length}</span>
          <span>${t("createdAt")}=${escapeHtml(formatDate(group.created_at))}</span>
          <span>${t("updatedAt")}=${escapeHtml(formatDate(group.updated_at))}</span>
        </div>
      </div>
      <div class="session-actions">
        <a class="button" href="/shared" data-nav="/shared">${t("back")}</a>
        <button type="button" data-action="open-shared-bind" data-group-id="${escapeAttr(group.id)}">${t("addHolding")}</button>
        <button type="button" data-action="run-sync-latest" data-group-id="${escapeAttr(group.id)}">${t("syncLatest")}</button>
        <button type="button" data-action="open-shared-rename" data-group-id="${escapeAttr(group.id)}" data-title="${escapeAttr(group.title || "")}">${t("rename")}</button>
        <button type="button" class="danger" data-action="open-shared-remove" data-group-id="${escapeAttr(group.id)}">${t("remove")}</button>
      </div>
    </section>
    <div class="shared-layout">
      <section class="section-panel stack">
        <div class="section-heading">
          <div>
            <strong>${t("holdings")}</strong>
            <small>${group.holdings.length}</small>
          </div>
        </div>
        <div class="shared-grid">
          ${group.holdings.map((holding) => renderHoldingCard(group, holding)).join("")}
        </div>
      </section>
      <aside class="detail-panel stack">
        ${renderMetaLine(t("provider"), sourceProvider?.name || group.source_provider)}
        ${renderMetaLine(t("sharedTitle"), group.title)}
        ${renderMetaLine(t("holdings"), String(group.holdings.length))}
        ${renderMetaLine(t("createdAt"), formatDate(group.created_at))}
        ${renderMetaLine(t("updatedAt"), formatDate(group.updated_at))}
      </aside>
    </div>`;
}

function renderHoldingCard(group, holding) {
  const provider = getOrderedProviders().find((item) => item.id === holding.provider);
  const sessionHref = `/sessions/${encodeURIComponent(holding.provider)}/${encodeURIComponent(holding.session_id)}`;
  return `
    <article class="binding-card">
      <header>
        <div>
          <strong>${escapeHtml(provider?.name || holding.provider)}</strong>
          <p class="modal-subtitle">${escapeHtml(holding.session_id)}</p>
        </div>
      </header>
      <div class="stack">
        ${renderMetaLine(t("workspace"), holding.target_dir)}
        ${renderMetaLine(t("lastActiveAt"), formatDate(holding.last_active_at))}
        ${renderMetaLine(t("lastSync"), formatDate(holding.last_sync_at))}
        ${renderMetaLine(t("syncFrom"), holding.last_sync_from)}
        ${renderMetaLine(t("error"), holding.last_error)}
      </div>
      <footer class="row-actions">
        <a class="button" href="${sessionHref}" data-nav="${sessionHref}">${t("openSession")}</a>
        <button type="button" data-action="open-sync-from" data-group-id="${escapeAttr(group.id)}" data-holding-id="${escapeAttr(holding.id)}" data-provider="${escapeAttr(
          provider?.name || holding.provider
        )}" data-session-id="${escapeAttr(holding.session_id)}">${t("syncFromThis")}</button>
        <button type="button" class="danger" data-action="open-unbind" data-group-id="${escapeAttr(group.id)}" data-holding-id="${escapeAttr(holding.id)}" data-provider="${escapeAttr(
          provider?.name || holding.provider
        )}" data-session-id="${escapeAttr(holding.session_id)}">${t("unbind")}</button>
      </footer>
    </article>`;
}

function renderModal() {
  if (!state.modal) {
    modalRoot.innerHTML = "";
    return;
  }
  const modal = state.modal;
  const formOpen = modal.kind === "form";
  modalRoot.innerHTML = `
    <div class="overlay">
      <div class="modal-card">
        <div class="modal-head">
          <div>
            <h3>${escapeHtml(modal.title)}</h3>
            ${modal.subtitle ? `<p class="muted">${escapeHtml(modal.subtitle)}</p>` : ""}
          </div>
          <button type="button" class="icon-button" data-action="close-modal">×</button>
        </div>
        ${formOpen
          ? `<form data-submit="${escapeAttr(modal.submit)}" class="modal-stack">
              ${modal.body}
              <div class="modal-actions">
                <button type="button" data-action="close-modal">${t("cancel")}</button>
                <button type="submit" class="${modal.submitClass || "invert"}">${escapeHtml(modal.submitLabel || t("save"))}</button>
              </div>
            </form>`
          : modal.body}
      </div>
    </div>`;
}

function renderLoading() {
  return `
    <div class="loading-layer ${state.loading ? "active" : ""}">
      <div class="loading-card">
        <span class="loading-spinner"></span>
        <span>${t("loading")}</span>
      </div>
    </div>`;
}

function renderToasts() {
  return `
    <div class="toast-stack">
      ${state.toasts
        .map(
          (item) => `
          <div class="toast ${item.error ? "error" : ""}">
            <h4>${escapeHtml(item.title)}</h4>
            <p>${escapeHtml(item.message)}</p>
          </div>`
        )
        .join("")}
    </div>`;
}

function closeModal() {
  state.modal = null;
  render();
}

function githubIcon() {
  return `<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.6 7.6 0 0 1 8 3.86c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>`;
}

function toast(title, message, error = false) {
  state.toasts = [...state.toasts, { title, message, error }].slice(-4);
  render();
  window.clearTimeout(toast.timer);
  toast.timer = window.setTimeout(() => {
    state.toasts = state.toasts.slice(-1);
    render();
  }, 3200);
}

function fatal(error) {
  appEl.innerHTML = `<div class="app-shell"><div class="empty-state"><h2>${escapeHtml(t("error"))}</h2><p>${escapeHtml(
    error.message || String(error)
  )}</p></div></div>`;
}

function filterAndSortGroups(groups) {
  const q = state.home.search.trim().toLowerCase();
  const visible = Math.max(1, Number(state.home.visible || 12));
  const showSubagents = state.meta.settings.show_opencode_subagents;
  return groups
    .map((group) => {
      let sessions = [...group.sessions];
      if (!showSubagents && group.provider_id === "opencode") {
        sessions = sessions.filter((item) => !isOpenCodeSubagentTitle(item.title));
      }
      if (q) {
        sessions = sessions.filter((item) =>
          [item.session_id, item.title, item.native_title, item.project_dir].some((value) =>
            String(value || "")
              .toLowerCase()
              .includes(q)
          )
        );
      }
      sessions.sort((left, right) => {
        if (state.home.sort === "title") {
          return String(left.title || left.session_id).localeCompare(String(right.title || right.session_id));
        }
        return (right.last_active_at || 0) - (left.last_active_at || 0);
      });
      return {
        ...group,
        total_sessions: sessions.length,
        shown_sessions: Math.min(sessions.length, visible),
        sessions: sessions.slice(0, visible),
      };
    })
    .filter((group) => group.sessions.length > 0);
}

function parseRoute(pathname) {
  if (pathname === "/shared") return { name: "shared-list" };
  const sessionMatch = pathname.match(/^\/sessions\/([^/]+)\/([^/]+)$/);
  if (sessionMatch) {
    return {
      name: "session",
      provider: decodeURIComponent(sessionMatch[1]),
      sessionId: decodeURIComponent(sessionMatch[2]),
    };
  }
  const sharedMatch = pathname.match(/^\/shared\/([^/]+)$/);
  if (sharedMatch) {
    return {
      name: "shared-detail",
      groupId: decodeURIComponent(sharedMatch[1]),
    };
  }
  return { name: "home" };
}

function findSharedRef(providerId, sessionId) {
  const groups = state.home.sharedGroups || [];
  const match = groups.find((group) =>
    (group.holdings || []).some((holding) => holding.provider === providerId && holding.session_id === sessionId)
  );
  return match?.id || null;
}

function isOpenCodeSubagentTitle(title) {
  if (!title) return false;
  return title.includes("(@") && title.includes(" subagent)");
}

function getOrderedProviders() {
  const order = state.meta.settings.agent_order || [];
  const providers = [...state.meta.providers];
  const indexMap = new Map(order.map((id, index) => [id, index]));
  providers.sort((left, right) => {
    const leftIndex = indexMap.has(left.id) ? indexMap.get(left.id) : Number.MAX_SAFE_INTEGER;
    const rightIndex = indexMap.has(right.id) ? indexMap.get(right.id) : Number.MAX_SAFE_INTEGER;
    if (leftIndex !== rightIndex) return leftIndex - rightIndex;
    return left.name.localeCompare(right.name);
  });
  return providers;
}

function getToolbarProviderCandidates() {
  const ordered = getOrderedProviders();
  const selected = ordered.filter((item) => state.home.providers.includes(item.id));
  if (selected.length) return selected;

  const fallbackIds = ["claude", "codex"];
  const fallback = fallbackIds
    .map((id) => ordered.find((item) => item.id === id))
    .filter(Boolean);
  return fallback.length ? fallback : ordered.slice(0, 2);
}

function getVisibleToolbarProviders() {
  const candidates = getToolbarProviderCandidates();
  if (!candidates.length) return [];

  const minVisible = state.home.providers.length ? 1 : Math.min(2, candidates.length);
  const storedCount = Number(state.ui.homeProviderVisibleCount || 0);
  const visibleCount = storedCount > 0 ? storedCount : minVisible;
  return candidates.slice(0, Math.max(minVisible, Math.min(visibleCount, candidates.length)));
}

let homeProviderLayoutFrame = 0;

function scheduleHomeProviderLayout() {
  window.cancelAnimationFrame(homeProviderLayoutFrame);
  homeProviderLayoutFrame = window.requestAnimationFrame(updateHomeProviderLayout);
}

function updateHomeProviderLayout() {
  if (state.route.name !== "home") return;

  const strip = document.querySelector(".home-provider-strip");
  if (!strip) return;

  const candidates = getToolbarProviderCandidates();
  if (!candidates.length) return;

  const moreWidth = measureToolbarControlWidth(t("more"), true);
  const available = strip.clientWidth;
  const minVisible = state.home.providers.length ? 1 : Math.min(2, candidates.length);

  let used = moreWidth;
  let visible = 0;
  for (const item of candidates) {
    const pillWidth = measureToolbarControlWidth(item.name, false);
    const nextUsed = used + (visible ? 8 : 0) + pillWidth;
    if (nextUsed <= available || visible < minVisible) {
      used = nextUsed;
      visible += 1;
      continue;
    }
    break;
  }

  const nextCount = Math.max(minVisible, Math.min(visible, candidates.length));
  if (state.ui.homeProviderVisibleCount !== nextCount) {
    state.ui.homeProviderVisibleCount = nextCount;
    render();
  }
}

function measureToolbarControlWidth(label, more = false) {
  const root = document.createElement(more ? "button" : "label");
  root.style.position = "fixed";
  root.style.left = "-10000px";
  root.style.top = "0";
  root.style.visibility = "hidden";
  root.style.pointerEvents = "none";

  if (more) {
    root.className = "agent-more-button";
    root.textContent = label;
  } else {
    root.className = "agent-pill";
    root.innerHTML = `<span>${escapeHtml(label)}</span>`;
  }

  document.body.append(root);
  const width = Math.ceil(root.getBoundingClientRect().width);
  root.remove();
  return width;
}

function getPrimaryProviders() {
  const ordered = getOrderedProviders();
  const primaryIds = state.meta.settings.primary_agents || [];
  const preferred = primaryIds.length
    ? ordered.filter((item) => primaryIds.includes(item.id))
    : ordered;
  return preferred.slice(0, 3);
}

function getFoldedProviders() {
  const ordered = getOrderedProviders();
  const visiblePrimary = new Set(getPrimaryProviders().map((item) => item.id));
  return ordered.filter((item) => !visiblePrimary.has(item.id));
}

function getDefaultSwitchTarget(sourceId) {
  const ordered = getOrderedProviders().filter((item) => item.id !== sourceId);
  if (!ordered.length) return "";
  if (sourceId === "codex") {
    const claude = ordered.find((item) => item.id === "claude");
    if (claude) return claude.id;
  }
  return ordered[0].id;
}

function sortSessionGroupsByDisplay(groups) {
  const order = getOrderedProviders().map((item) => item.id);
  const indexMap = new Map(order.map((id, index) => [id, index]));
  return [...groups].sort((left, right) => {
    const leftIndex = indexMap.has(left.provider_id) ? indexMap.get(left.provider_id) : Number.MAX_SAFE_INTEGER;
    const rightIndex = indexMap.has(right.provider_id) ? indexMap.get(right.provider_id) : Number.MAX_SAFE_INTEGER;
    if (leftIndex !== rightIndex) return leftIndex - rightIndex;
    return left.provider_name.localeCompare(right.provider_name);
  });
}

function providerOptions(skipId = "", selectedId = "") {
  return getOrderedProviders()
    .filter((item) => item.id !== skipId)
    .map(
      (item) =>
        `<option value="${escapeAttr(item.id)}"${item.id === selectedId ? " selected" : ""}>${escapeHtml(item.name)}</option>`
    )
    .join("");
}

function renderPathField(name, label, value, hint, placeholder = "") {
  return `
    <label class="field">
      <span>${label}</span>
      <div class="path-picker">
        <input name="${escapeAttr(name)}" list="known-workspaces" value="${escapeAttr(value || "")}" placeholder="${escapeAttr(
          placeholder
        )}">
        <button type="button" class="ghost" data-action="browse-folder" data-target-field="${escapeAttr(name)}">${t(
          "browse"
        )}</button>
      </div>
      ${hint ? `<small class="muted">${hint}</small>` : ""}
    </label>`;
}

function renderWorkspaceDatalist() {
  const items = state.meta?.workspaces || [];
  if (!items.length) return "";
  return `<datalist id="known-workspaces">${items
    .map((item) => `<option value="${escapeAttr(item.path)}"></option>`)
    .join("")}</datalist>`;
}

async function browseFolderForField(trigger) {
  const fieldName = trigger?.dataset?.targetField;
  if (!fieldName) return;

  const scope = trigger.closest("form, .workspace-panel, .modal-card") || document;
  const selector = `[name="${fieldName}"]`;
  const input = scope.querySelector(selector) || document.querySelector(selector);
  if (!(input instanceof HTMLInputElement)) return;

  try {
    const result = await api("/api/v1/system/select-folder", {
      method: "POST",
      body: {
        start_path: emptyToNull(input.value) || state.home.workspace || null,
      },
    });
    if (result.path) {
      input.value = result.path;
    }
  } catch (error) {
    const message = /only available in the desktop app/i.test(error.message)
      ? t("folderPickerUnavailable")
      : error.message;
    toast(t("error"), message, true);
  }
}

function shortId(value) {
  const text = String(value || "");
  if (text.length <= 12) return text;
  return `${text.slice(0, 8)}...`;
}

function renderMetaLine(label, value) {
  if (value === null || value === undefined || value === "") return "";
  return `<div class="stack"><span class="eyebrow">${escapeHtml(label)}</span><div class="path-line">${escapeHtml(
    formatValue(value)
  )}</div></div>`;
}

function markdown(text) {
  const lines = String(text || "").split("\n");
  const chunks = [];
  let inCode = false;
  let codeLines = [];

  for (const line of lines) {
    if (line.startsWith("```")) {
      if (inCode) {
        chunks.push(`<pre class="code-block">${escapeHtml(codeLines.join("\n"))}</pre>`);
        codeLines = [];
        inCode = false;
      } else {
        inCode = true;
      }
      continue;
    }

    if (inCode) {
      codeLines.push(line);
      continue;
    }

    if (!line.trim()) {
      chunks.push("<p><br></p>");
      continue;
    }

    chunks.push(`<p>${escapeHtml(line).replace(/`([^`]+)`/g, '<span class="inline-code">$1</span>')}</p>`);
  }

  if (inCode) {
    chunks.push(`<pre class="code-block">${escapeHtml(codeLines.join("\n"))}</pre>`);
  }

  return chunks.join("");
}

function formatDate(value) {
  if (!value) return "—";
  const date = typeof value === "number" ? new Date(value) : new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString(lang() === "zh" ? "zh-CN" : "en-US");
}

function formatValue(value) {
  if (typeof value === "string" && /^\d{4}-\d{2}-\d{2}T/.test(value)) return formatDate(value);
  return String(value);
}

function formatBytes(value) {
  if (value === null || value === undefined || value === "") return "—";
  const units = ["B", "KB", "MB", "GB"];
  let size = Number(value);
  if (!Number.isFinite(size)) return "—";
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function workspaceName(path) {
  if (!path) return "";
  return path.replace(/[\\/]$/, "").split(/[\\/]/).pop() || path;
}

function emptyToNull(value) {
  const text = String(value || "").trim();
  return text ? text : null;
}

function numberOrNull(value) {
  const text = String(value || "").trim();
  if (!text) return null;
  const number = Number(text);
  return Number.isFinite(number) ? number : null;
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function escapeAttr(value) {
  return escapeHtml(String(value ?? "")).replaceAll("'", "&#39;");
}

function lang() {
  return state.meta?.settings?.language || "zh";
}

function t(key) {
  return I18N[lang()]?.[key] || I18N.zh[key] || key;
}

async function loadI18n() {
  const response = await fetch("/i18n.json", {
    headers: {
      Accept: "application/json",
    },
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  I18N = await response.json();
  setDocumentLanguage();
}

function setDocumentLanguage() {
  document.documentElement.lang = lang() === "zh" ? "zh-CN" : "en";
}

function setLoading(active) {
  state.loading += active ? 1 : -1;
  if (state.loading < 0) state.loading = 0;
  render();
}
