const ASCII = `███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████  ██    ██  ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██  ██    ██  ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██  ██    ██  ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██`;

let I18N = { zh: {}, en: {} };
const REPOSITORY_URL = "https://github.com/ip2a/memorph";
const NPM_PACKAGE_URL = "https://www.npmjs.com/package/memorph";
const CRATES_PACKAGE_URL = "https://crates.io/crates/memorph";
const PYPI_PACKAGE_URL = "https://pypi.org/project/memorph/";
const ABOUT_LINKS = [
  {
    label: "GitHub",
    url: REPOSITORY_URL,
    iconUrl: "https://github.com/favicon.ico",
  },
  {
    label: "npm",
    url: NPM_PACKAGE_URL,
    iconUrl: "https://www.npmjs.com/favicon.ico",
  },
  {
    label: "crates.io",
    url: CRATES_PACKAGE_URL,
    iconUrl: "https://crates.io/favicon.ico",
  },
  {
    label: "PyPI",
    url: PYPI_PACKAGE_URL,
    iconUrl: "https://pypi.org/favicon.ico",
  },
];
const ASCII_BANNER_COLORS = ["#7dd3fc", "#86efac", "#f0abfc", "#facc15", "#fb7185", "#c4b5fd"];

const state = {
  meta: null,
  route: parseRoute(window.location.pathname),
  loading: 0,
  loadingInfo: null,
  ui: {
    homeProviderVisibleCount: null,
    asciiBannerColor: randomAsciiBannerColor(),
    settingsSection: "general",
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
  manager: {
    draft: null,
    preview: null,
    report: null,
    pendingItems: [],
  },
  compression: {
    archives: [],
    providers: [],
  },
  agents: {
    providers: [],
    selectedProvider: "",
    settingResults: {},
    pendingSettings: {},
  },
  updateCheck: {
    checking: false,
    result: null,
    error: "",
  },
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

  const externalLink = event.target.closest('a[href]');
  if (externalLink) {
    const href = externalLink.getAttribute("href") || "";
    if (isExternalHttpUrl(href)) {
      event.preventDefault();
      void openExternalUrl(href);
      return;
    }
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
  if (target.dataset.role === "agent-setting-toggle") {
    void updateAgentSetting(target.dataset.provider, target.dataset.settingId, target.checked);
  }
  if (target.dataset.role === "manager-provider-toggle") {
    void updateManagerProvider(target.value, target.checked, target);
  }
  if (target.dataset.role === "home-search") {
    state.home.search = target.value;
    render();
  }
  if (target.dataset.role === "select-all-manager") {
    document
      .querySelectorAll('input[name="manager_item"]')
      .forEach((el) => (el.checked = target.checked));
    updateManagerSelectionStats();
  }
  if (target.name === "manager_item") {
    updateManagerSelectionStats();
  }
});

void bootstrap();

async function bootstrap() {
  setLoading(true, { label: t("loadingWorkspace"), detail: workspaceName(state.home.workspace) });
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
  if (state.route.name === "agents" && window.location.pathname === "/tools") {
    replacePath("/agents");
  }
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
    } else if (route.name === "manager") {
      state.home.sharedGroups = await api("/api/v1/share/status");
      if (!state.manager.preview) {
        await loadDefaultManagerPreview();
      }
    } else if (route.name === "compression") {
      const [archives, providers] = await Promise.all([
        api("/api/v1/compression/archives"),
        api("/api/v1/compression/providers"),
      ]);
      state.compression.archives = archives;
      state.compression.providers = providers;
    } else if (route.name === "agents") {
      await loadAgentManagement();
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
    details: "false",
  });
  state.home.groups = await api(`/api/v1/sessions?${params.toString()}`);
  state.home.sharedGroups = await api("/api/v1/share/status");
}

async function loadAgentManagement() {
  const payload = await api("/api/v1/agents");
  state.agents.providers = payload.providers || [];
  if (
    !state.agents.selectedProvider ||
    !state.agents.providers.some((item) => item.provider_id === state.agents.selectedProvider)
  ) {
    state.agents.selectedProvider = state.agents.providers[0]?.provider_id || "";
  }
}

async function detectAgentProvider(providerId) {
  if (!providerId) return;
  const provider = await api(`/api/v1/agents/${encodeURIComponent(providerId)}/detect`, {
    method: "POST",
  });
  const index = state.agents.providers.findIndex((item) => item.provider_id === provider.provider_id);
  if (index >= 0) {
    state.agents.providers[index] = provider;
  } else {
    state.agents.providers.push(provider);
  }
  state.agents.selectedProvider = provider.provider_id;
  render();
}

function refreshSettingsModalWithCurrentDraft() {
  if (state.modal?.submit !== "save-settings") {
    render();
    return;
  }
  openSettingsModal(readSettingsDraft());
}

function switchSettingsSection(section) {
  if (!new Set(["general", "display", "order", "config", "about"]).has(section)) return;
  state.ui.settingsSection = section;
  openSettingsModal(readSettingsDraft());
}

async function runUpdateCheck() {
  state.updateCheck.checking = true;
  state.updateCheck.error = "";
  refreshSettingsModalWithCurrentDraft();
  try {
    state.updateCheck.result = await api("/api/v1/update-check");
  } catch (error) {
    state.updateCheck.result = null;
    state.updateCheck.error = error.message;
  } finally {
    state.updateCheck.checking = false;
    refreshSettingsModalWithCurrentDraft();
  }
}

function randomAsciiBannerColor() {
  return ASCII_BANNER_COLORS[Math.floor(Math.random() * ASCII_BANNER_COLORS.length)];
}

function navigate(path) {
  const samePath = window.location.pathname === path;
  state.modal = null;
  if (!samePath) {
    history.pushState({}, "", path);
  }
  state.route = parseRoute(path);
  if (state.route.name === "home") {
    state.ui.asciiBannerColor = randomAsciiBannerColor();
  }
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
  setLoading(true, { label: t("loadingWorkspace"), detail: workspaceName(workspace) });
  state.home.workspace = workspace;
  try {
    if (state.route.name === "manager") {
      state.manager = { draft: defaultManagerDraft(), preview: null, report: null, pendingItems: [] };
    }
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
    if (state.route.name === "agents") {
      await loadAgentManagement();
    }
    await refreshWorkspaceMeta();
    render();
  } finally {
    setLoading(false);
  }
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
  if (state.modal?.view === "agent-filter") {
    openAgentFilterModal();
  } else {
    render();
  }
  void persistProvidersAndReload();
}

async function updateManagerProvider(provider, checked, trigger = null) {
  const draft = state.manager.draft || defaultManagerDraft();
  const next = new Set(draft.providers);
  if (checked) next.add(provider);
  else next.delete(provider);
  state.manager.draft = {
    ...draft,
    providers: [...next],
  };

  const item = trigger?.closest?.(".manager-provider-item");
  if (!item) {
    render();
  } else {
    item.classList.toggle("is-active", checked);
    const stateMarker = item.querySelector(".agent-provider-state");
    if (stateMarker) {
      stateMarker.classList.toggle("is-installed", checked);
      stateMarker.classList.toggle("is-missing", !checked);
      stateMarker.textContent = checked ? "●" : "○";
    }
  }

  if (state.manager.preview?.default_preview) {
    await loadDefaultManagerPreview();
  }
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
    case "open-external":
      await openExternalUrl(data.url || trigger?.getAttribute("href") || "");
      break;
    case "check-update":
      await runUpdateCheck();
      break;
    case "open-settings":
      openSettingsModal();
      break;
    case "switch-settings-section":
      switchSettingsSection(data.section || "general");
      break;
    case "open-agents":
      navigate("/agents");
      break;
    case "open-manager":
      navigate("/manager");
      break;
    case "detect-agent-provider":
      await detectAgentProvider(data.provider);
      break;
    case "refresh-agents":
      await loadAgentManagement();
      render();
      break;
    case "select-agent-provider":
      state.agents.selectedProvider = data.provider || "";
      render();
      break;
    case "run-agent-setting":
      await runAgentSetting(data.provider, data.settingId);
      break;
    case "open-repaired-sessions":
      openRepairedSessionsModal();
      break;
    case "open-compression":
      navigate("/compression");
      break;
    case "open-compression-restore":
      openCompressionRestoreModal(data.archiveRef || "");
      break;
    case "open-compression-expand":
      openCompressionExpandModal();
      break;
    case "refresh-compression":
      {
        const [archives, providers] = await Promise.all([
          api("/api/v1/compression/archives"),
          api("/api/v1/compression/providers"),
        ]);
        state.compression.archives = archives;
        state.compression.providers = providers;
      }
      toast(t("refreshed"), t("compressionArchives"));
      render();
      break;
    case "open-import":
      openImportModal();
      break;
    case "open-manager-filter":
      openManagerFilterModal();
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
    case "copy-detail-message":
      await copyDetailMessage(Number(data.messageIndex));
      break;
    case "delete-detail-message":
      toast(t("remove"), t("notImplemented"));
      break;
    case "toggle-detail-message":
      toggleDetailMessage(trigger);
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
    case "open-manager-clean-confirm":
      openManagerCleanConfirmModal();
      break;
    case "open-manager-backup-confirm":
      openManagerBackupConfirmModal();
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
    case "close-toast":
      closeToast(Number(data.toastIndex));
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
        await runManagerClean();
        break;
      case "backup-manager":
        await runManagerBackup();
        break;
      case "restore-compression":
        await runCompressionRestore(formData);
        break;
      case "expand-compression":
        await runCompressionExpand(formData);
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
    view: "agent-filter",
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

function openCompressionRestoreModal(archiveRef) {
  const defaultPrefix = archiveRef
    .replace(/^memorph-archive:\/\//, "")
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  state.modal = {
    kind: "form",
    title: t("restoreCompressionArchive"),
    submit: "restore-compression",
    body: `
      <input type="hidden" name="archive_ref" value="${escapeAttr(archiveRef)}">
      <div class="stack">
        <div class="verify-block">
          <span class="block-label">${t("archiveRef")}</span>
          <div class="path-line">${escapeHtml(archiveRef)}</div>
        </div>
        <label class="field">
          <span>${t("outputPrefix")}</span>
          <input name="output_prefix" value="${escapeAttr(defaultPrefix || "compression_archive")}">
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
    submitLabel: t("restore"),
  };
  render();
}

function openCompressionExpandModal() {
  state.modal = {
    kind: "form",
    title: t("expandCompressionSession"),
    submit: "expand-compression",
    body: `
      <div class="stack">
        <label class="field">
          <span>${t("sessionFile")}</span>
          <input name="file" placeholder="${escapeAttr(t("fileOrIdPlaceholder"))}">
        </label>
        <label class="field">
          <span>${t("outputPrefix")}</span>
          <input name="output_prefix" placeholder="session_expanded">
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
    submitLabel: t("expand"),
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

function renderUpdateCheckSummary() {
  if (state.updateCheck.error) {
    return `<span class="settings-update-status is-error">${escapeHtml(`${t("updateCheckFailed")}: ${state.updateCheck.error}`)}</span>`;
  }
  if (!state.updateCheck.result) return "";

  const result = state.updateCheck.result;
  const statusText = result.has_update ? t("updateAvailableStatus") : t("upToDateStatus");
  const summaryText = `${t("installSource")}: ${result.install_source_label} · ${t("latestVersionLabel")}: v${result.latest_version}`;
  const commandText = `${t("updateCommandLabel")}: ${result.update_command}`;

  return `
    <span class="settings-update-status ${result.has_update ? "is-available" : ""}">${escapeHtml(statusText)}</span>
    <span class="settings-update-meta">${escapeHtml(summaryText)}</span>
    <span class="settings-update-command">${escapeHtml(commandText)}</span>`;
}

function renderAboutLinks() {
  return ABOUT_LINKS.map(
    (item) => `
      <a class="settings-about-link" href="${escapeAttr(item.url)}" target="_blank" rel="noopener noreferrer" data-action="open-external" data-url="${escapeAttr(item.url)}">
        <span class="settings-about-link-icon" aria-hidden="true">
          <img src="${escapeAttr(item.iconUrl)}" alt="">
        </span>
        <span class="settings-about-link-copy">
          <strong>${escapeHtml(item.label)}</strong>
          <span>${escapeHtml(item.url)}</span>
        </span>
      </a>`
  ).join("");
}

function openSettingsModal(draft = null) {
  const settings = draft || state.meta.settings;
  const activeSection = state.ui.settingsSection || "general";
  const settingsSectionClass = (section) => `settings-section ${activeSection === section ? "is-active" : "is-hidden"}`;
  const settingsNavClass = (section) => `settings-nav-item ${activeSection === section ? "is-active" : ""}`;
  const items = [...settings.agent_order]
    .map((providerId, index) => {
      const info = state.meta.providers.find((item) => item.id === providerId);
      const agentEntry = state.agents.providers.find((item) => item.provider_id === providerId);
      const primary = settings.primary_agents.includes(providerId);
      const installed = agentEntry?.installed ?? false;
      return `
        <div class="settings-provider-row">
          <div class="settings-copy">
            <div class="settings-provider-name">
              <strong>${escapeHtml(info?.name || providerId)}</strong>
              <span class="settings-provider-status ${installed ? "is-installed" : "is-missing"}">${installed ? t("installed") : t("notDetected")}</span>
            </div>
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
    className: "settings-modal-card",
    actionsInHead: true,
    body: `
      <div class="settings-layout">
        <nav class="settings-sidebar" aria-label="${escapeAttr(t("settingsTitle"))}">
          <button type="button" class="${settingsNavClass("general")}" data-action="switch-settings-section" data-section="general">${t("general")}</button>
          <button type="button" class="${settingsNavClass("display")}" data-action="switch-settings-section" data-section="display">${t("display")}</button>
          <button type="button" class="${settingsNavClass("order")}" data-action="switch-settings-section" data-section="order">${t("order")}</button>
          <button type="button" class="${settingsNavClass("config")}" data-action="switch-settings-section" data-section="config">${t("configFile")}</button>
          <button type="button" class="${settingsNavClass("about")}" data-action="switch-settings-section" data-section="about">${t("about")}</button>
        </nav>
        <div class="settings-content">
          <section class="${settingsSectionClass("general")}" id="settings-general">
            <div class="settings-section-head">
              <h4>${t("general")}</h4>
            </div>
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
                  <strong>${t("defaultBackupDir")}</strong>
                  <span>${t("defaultBackupDirHint")}</span>
                </div>
                <div class="path-picker">
                  <input name="default_backup_dir" list="known-workspaces" value="${escapeAttr(
                    settings.default_backup_dir || "./backups"
                  )}" placeholder="./backups">
                  <button type="button" class="ghost" data-action="browse-folder" data-target-field="default_backup_dir">${t(
                    "browse"
                  )}</button>
                </div>
              </div>
              <div class="settings-row">
                <div class="settings-copy">
                  <strong>${t("logSettings")}</strong>
                  <span>${t("logSettingsHint")}</span>
                </div>
                <div class="settings-agent-list">
                  <label class="field compact-number-field">
                    <span>${t("logMaxSizeMb")}</span>
                    <input type="text" inputmode="decimal" name="log_max_size_mb" value="${escapeAttr(
                      String(((settings.logging?.max_size_bytes || 5 * 1024 * 1024) / 1024 / 1024).toFixed(1).replace(/\.0$/, ""))
                    )}">
                  </label>
                  <label class="field compact-number-field">
                    <span>${t("logRetentionDays")}</span>
                    <input type="text" inputmode="numeric" name="log_retention_days" value="${escapeAttr(
                      settings.logging?.retention_days == null ? "" : String(settings.logging.retention_days)
                    )}" placeholder="${escapeAttr(t("unlimited"))}">
                  </label>
                </div>
              </div>
            </div>
          </section>
          <section class="${settingsSectionClass("display")}" id="settings-display">
            <div class="settings-section-head">
              <h4>${t("display")}</h4>
            </div>
            <div class="settings-list">
              <div class="settings-row">
                <div class="settings-copy">
                  <strong>${t("sessionsPerProvider")}</strong>
                  <span>${t("settingsSessionsHint")}</span>
                </div>
                <input type="number" min="1" max="200" name="sessions_per_provider" value="${settings.sessions_per_provider}">
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
            </div>
          </section>
          <section class="${settingsSectionClass("order")}" id="settings-order">
            <div class="settings-section-head">
              <h4>${t("order")}</h4>
            </div>
            <div class="settings-list">
              <div class="settings-row settings-row-stacked">
                <div class="settings-copy">
                  <strong>${t("providers")}</strong>
                  <span>${t("settingsProvidersHint")}</span>
                </div>
                <div class="settings-provider-list settings-provider-list-vertical">${items}</div>
              </div>
            </div>
          </section>
          <section class="${settingsSectionClass("config")}" id="settings-config">
            <div class="settings-section-head">
              <h4>${t("configFile")}</h4>
            </div>
            <div class="settings-list">
              <div class="settings-row settings-row-stacked">
                <div class="settings-copy">
                  <strong>${escapeHtml(state.meta?.config_file?.path || "")}</strong>
                  <span>${escapeHtml(state.meta?.config_file?.format || "json")}</span>
                </div>
                <pre class="settings-config-content"><code>${escapeHtml(state.meta?.config_file?.content || "")}</code></pre>
              </div>
            </div>
          </section>
          <section class="${settingsSectionClass("about")}" id="settings-about">
            <div class="settings-section-head">
              <h4>${t("about")}</h4>
            </div>
            <div class="settings-list">
              <div class="settings-row settings-row-stacked settings-about-row">
                <div class="settings-about-head">
                  <div class="settings-copy">
                    <strong>${t("version")}</strong>
                    <span>v${escapeHtml(state.meta?.version || "")}</span>
                  </div>
                  <button type="button" data-action="check-update" ${state.updateCheck.checking ? "disabled" : ""}>
                    ${state.updateCheck.checking ? t("checkingUpdate") : t("checkUpdate")}
                  </button>
                </div>
                <div class="settings-about-links">
                  ${renderAboutLinks()}
                </div>
                <div class="settings-about-update">
                  ${renderUpdateCheckSummary()}
                </div>
              </div>
            </div>
          </section>
        </div>
      </div>`,
    submitLabel: t("save"),
  };
  render();
}

function renderManagerPreview(preview, report = null) {
  const rows = preview.items
    .map((item) => {
      const encoded = escapeAttr(encodeURIComponent(JSON.stringify(item)));
      const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
      return `
        <article class="manager-row">
          <div class="manager-row-head">
            <div class="manager-row-copy">
              <a class="manager-title-link" href="${href}" data-nav="${href}">${escapeHtml(item.title || item.session_id)}</a>
              <div class="manager-meta">
                <span>${escapeHtml(item.provider_name)}</span>
                <span>${escapeHtml(formatBytes(item.size_bytes))}</span>
                <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(item.last_active_at)))}</span>
              </div>
            </div>
            <label class="manager-select">
              <input type="checkbox" name="manager_item" value="${encoded}">
            </label>
          </div>
        </article>`;
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
        <span class="manager-selection-summary" data-role="manager-selection-summary">
          ${managerSelectionSummary(preview.total_count, 0, 0)}
        </span>
      </div>
      <div class="manager-preview-actions">
        <button type="button" class="invert" data-action="open-manager-filter">${t("filters")}</button>
        <button type="button" class="danger" data-action="open-manager-clean-confirm">${t("cleanSelected")}</button>
        <button type="button" data-action="open-manager-backup-confirm">${t("backupSelected")}</button>
        <label class="check-row">
          <input type="checkbox" data-role="select-all-manager">
          <span>${t("selectAll")}</span>
        </label>
      </div>
    </div>
    <div class="manager-list">${rows || `<div class="empty-state">${t("emptySessions")}</div>`}</div>`;
}

function managerSelectionSummary(total, selected, bytes) {
  return t("managerSelectionSummary")
    .replace("{count}", String(total))
    .replace("{selected}", String(selected))
    .replace("{size}", formatBytes(bytes));
}

function updateManagerSelectionStats() {
  const preview = state.manager.preview;
  const summary = document.querySelector('[data-role="manager-selection-summary"]');
  if (!preview || !summary) return;
  const selected = selectedManagerItems();
  const bytes = selected.reduce((sum, item) => sum + Number(item.size_bytes || 0), 0);
  summary.textContent = managerSelectionSummary(preview.total_count, selected.length, bytes);
  const all = [...document.querySelectorAll('input[name="manager_item"]')];
  const selectAll = document.querySelector('input[data-role="select-all-manager"]');
  if (selectAll) {
    selectAll.checked = all.length > 0 && selected.length === all.length;
    selectAll.indeterminate = selected.length > 0 && selected.length < all.length;
  }
}

function openManagerCleanConfirmModal() {
  const items = selectedManagerItems();
  if (!items.length) {
    toast(t("error"), t("noSelection"), true);
    return;
  }
  state.manager.pendingItems = items;
  state.modal = {
    kind: "form",
    title: t("cleanSelected"),
    submit: "clean-manager",
    submitClass: "danger",
    submitLabel: t("confirm"),
    body: `
      <div class="stack">
        <p>${t("cleanConfirm")}</p>
        <div class="path-line">${t("sessionsStat")}: ${items.length}</div>
      </div>`,
  };
  render();
}

function openManagerBackupConfirmModal() {
  const items = selectedManagerItems();
  if (!items.length) {
    toast(t("error"), t("noSelection"), true);
    return;
  }
  state.manager.pendingItems = items;
  const outputDir = state.meta.settings.default_backup_dir || "./backups";
  state.modal = {
    kind: "form",
    title: t("backupSelected"),
    submit: "backup-manager",
    submitLabel: t("confirm"),
    body: `
      <div class="stack">
        <p>${t("backupConfirm")}</p>
        <div class="path-line">${t("sessionsStat")}: ${items.length}</div>
        <div class="path-line">${t("backupDir")}: ${escapeHtml(outputDir)}</div>
      </div>`,
  };
  render();
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

async function runCompressionRestore(formData) {
  const result = await api("/api/v1/compression/restore", {
    method: "POST",
    body: {
      archive_ref: String(formData.get("archive_ref")),
      output_prefix: emptyToNull(formData.get("output_prefix")),
      format: String(formData.get("format")),
    },
  });
  closeModal();
  openActionResultModal({
    title: t("restoreComplete"),
    lines: result.files,
  });
}

async function runCompressionExpand(formData) {
  const file = String(formData.get("file") || "").trim();
  if (!file) throw new Error(t("fileRequired"));
  const result = await api("/api/v1/compression/expand", {
    method: "POST",
    body: {
      file,
      output_prefix: emptyToNull(formData.get("output_prefix")),
      format: String(formData.get("format")),
    },
  });
  closeModal();
  openActionResultModal({
    title: t("expandComplete"),
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
    show_opencode_subagents: Boolean(state.meta.settings.show_opencode_subagents),
    default_backup_dir: String(formData.get("default_backup_dir") || "./backups"),
    logging: logSettingsFromFormData(formData),
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
  await api("/api/v1/settings", {
    method: "PUT",
    body,
  });
  state.meta = await api("/api/v1/meta");
  setDocumentLanguage();
  state.home.visible = state.meta.settings.sessions_per_provider;
  toast(t("saved"), t("settingsTitle"));
  render();
}

async function loadDefaultManagerPreview() {
  const draft = {
    ...(state.manager.draft || defaultManagerDraft()),
    is_default_preview: true,
  };
  if (!draft.providers.length) {
    const preview = emptyManagerPreview();
    preview.default_preview = true;
    state.manager = { draft, preview, report: null, pendingItems: [] };
    render();
    return;
  }
  const preview = await api("/api/v1/manager/preview", {
    method: "POST",
    body: managerPreviewBody(draft),
  });
  preview.output_dir = "";
  preview.default_preview = true;
  state.manager = { draft, preview, report: null, pendingItems: [] };
  render();
}

async function runManagerPreview(formData) {
  const draft = managerDraftFromFormData(formData);
  if (!draft.providers.length) throw new Error(t("noTargetAgentSelected"));
  setLoading(true, { label: t("managerPreview"), detail: t("scanningSessions") });
  try {
    const preview = await api("/api/v1/manager/preview", {
      method: "POST",
      body: managerPreviewBody(draft),
    });
    applyManagerDraftFilters(preview, draft);
    preview.output_dir = "";
    preview.default_preview = false;
    state.manager = { draft, preview, report: null, pendingItems: [] };
    state.modal = null;
    if (state.route.name !== "manager") replacePath("/manager");
    render();
  } finally {
    setLoading(false);
  }
}

async function runManagerClean() {
  const draft = readManagerDraft();
  const items = state.manager.pendingItems.length ? state.manager.pendingItems : selectedManagerItems();
  if (!items.length) throw new Error(t("noSelection"));
  closeModal();
  setLoading(true, { label: t("cleanSelected"), detail: `${items.length} ${t("sessionsStat")}`, progress: 0.12 });
  try {
    const result = await api("/api/v1/manager/clean", {
      method: "POST",
      body: { items },
    });
    updateLoading({ detail: t("refreshingSessions"), progress: 0.82 });
    const preview = await api("/api/v1/manager/preview", {
      method: "POST",
      body: managerPreviewBody(draft),
    });
    applyManagerDraftFilters(preview, draft);
    preview.output_dir = "";
    preview.default_preview = Boolean(draft.is_default_preview);
    state.manager = { draft, preview, report: null, pendingItems: [] };
    openActionResultModal({
      title: t("cleanSelected"),
      summary: `${result.success} ${t("managerCleaned")}, ${result.failed} ${t("managerFailed")}, ${formatBytes(
        result.freed_bytes
      )} ${t("managerFreed")}`,
      lines: result.errors || [],
    });
  } finally {
    setLoading(false);
  }
}

async function runManagerBackup() {
  const draft = readManagerDraft();
  const items = state.manager.pendingItems.length ? state.manager.pendingItems : selectedManagerItems();
  if (!items.length) throw new Error(t("noSelection"));
  const outputDir = state.meta.settings.default_backup_dir || "./backups";
  closeModal();
  setLoading(true, { label: t("backupSelected"), detail: `${items.length} ${t("sessionsStat")}`, progress: 0.12 });
  try {
    const result = await api("/api/v1/manager/backup", {
      method: "POST",
      body: {
        items,
        output_dir: outputDir,
      },
    });
    updateLoading({ detail: t("refreshingSessions"), progress: 0.82 });
    const preview = await api("/api/v1/manager/preview", {
      method: "POST",
      body: managerPreviewBody(draft),
    });
    applyManagerDraftFilters(preview, draft);
    preview.output_dir = outputDir;
    preview.default_preview = Boolean(draft.is_default_preview);
    state.manager = { draft, preview, report: null, pendingItems: [] };
    openActionResultModal({
      title: t("backupSelected"),
      summary: `${result.success} ${t("managerExported")}, ${result.failed} ${t("managerFailed")}`,
      lines: [...(result.files || []), ...(result.errors || [])],
    });
  } finally {
    setLoading(false);
  }
}

function selectedManagerItems() {
  return [...document.querySelectorAll('input[name="manager_item"]:checked')].map((el) =>
    JSON.parse(decodeURIComponent(el.value))
  );
}

function defaultManagerDraft() {
  const selectedProviders = state.home.providers.length
    ? state.home.providers
    : state.meta
      ? getOrderedProviders().map((item) => item.id)
      : [];
  return {
    workspace: state.home.workspace || "",
    older_than_days: "",
    older_than_unit: "days",
    size_min_value: "",
    size_min_unit: "mb",
    size_max_value: "",
    size_max_unit: "mb",
    title_contains: "",
    title_excludes: "",
    max_results: "10",
    sort_order: "recent",
    providers: selectedProviders,
    is_default_preview: true,
  };
}

function managerDraftFromFormData(formData) {
  const current = state.manager.draft || defaultManagerDraft();
  return {
    workspace: state.home.workspace || "",
    older_than_days: String(formData.get("older_than_days") || ""),
    older_than_unit: String(formData.get("older_than_unit") || "days"),
    size_min_value: String(formData.get("size_min_value") || ""),
    size_min_unit: String(formData.get("size_min_unit") || "mb"),
    size_max_value: String(formData.get("size_max_value") || ""),
    size_max_unit: String(formData.get("size_max_unit") || "mb"),
    title_contains: String(formData.get("title_contains") || ""),
    title_excludes: String(formData.get("title_excludes") || ""),
    max_results: String(formData.get("max_results") || ""),
    sort_order: String(formData.get("sort_order") || current.sort_order || "recent"),
    providers: formData.getAll("providers").map(String).length
      ? formData.getAll("providers").map(String)
      : current.providers,
    is_default_preview: false,
  };
}

function readManagerDraft() {
  const form = document.querySelector('form[data-submit="preview-manager"]');
  if (!form) return state.manager.draft || defaultManagerDraft();
  return managerDraftFromFormData(new FormData(form));
}

function managerPreviewBody(draft) {
  const minSizeBytes = managerSizeBytesValue(draft.size_min_value, draft.size_min_unit);
  const maxSizeBytes = managerSizeBytesValue(draft.size_max_value, draft.size_max_unit);
  const olderThanMs = managerAgeMsValue(draft.older_than_days, draft.older_than_unit);
  return {
    workspace: emptyToNull(draft.workspace),
    older_than_ms: olderThanMs,
    ...(minSizeBytes === null ? {} : { larger_than_bytes: minSizeBytes }),
    ...(maxSizeBytes === null ? {} : { smaller_than_bytes: maxSizeBytes }),
    sort: draft.sort_order || "recent",
    limit: managerLimitValue(draft.max_results),
    providers: draft.providers,
  };
}

function applyManagerDraftFilters(preview, draft) {
  const includes = normalizeSearchTerm(draft.title_contains);
  const excludes = normalizeSearchTerm(draft.title_excludes);
  if (!includes && !excludes) return preview;

  preview.items = preview.items.filter((item) => {
    const title = normalizeSearchTerm(item.title || item.session_id || "");
    if (includes && !title.includes(includes)) return false;
    if (excludes && title.includes(excludes)) return false;
    return true;
  });
  preview.total_count = preview.items.length;
  preview.total_size_bytes = preview.items.reduce((sum, item) => sum + Number(item.size_bytes || 0), 0);
  return preview;
}

function normalizeSearchTerm(value) {
  return String(value || "").trim().toLocaleLowerCase();
}

function managerLimitValue(value) {
  const amount = numberOrNull(value);
  if (amount === null) return 10;
  return Math.max(1, Math.floor(amount));
}

function managerAgeMsValue(value, unit) {
  const amount = numberOrNull(value);
  if (amount === null) return null;
  const minute = 60 * 1000;
  if (unit === "minutes") return Math.ceil(Date.now() - amount * minute);
  if (unit === "hours") return Math.ceil(Date.now() - amount * 60 * minute);
  if (unit === "weeks") return Math.ceil(Date.now() - amount * 7 * 24 * 60 * minute);
  if (unit === "months") return Math.ceil(Date.now() - amount * 30 * 24 * 60 * minute);
  return Math.ceil(Date.now() - amount * 24 * 60 * minute);
}

function managerSizeBytesValue(value, unit) {
  const amount = numberOrNull(value);
  if (amount === null) return null;
  if (unit === "kb") return Math.ceil(amount * 1024);
  if (unit === "gb") return Math.ceil(amount * 1024 * 1024 * 1024);
  return Math.ceil(amount * 1024 * 1024);
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
    show_opencode_subagents: Boolean(state.meta.settings.show_opencode_subagents),
    default_backup_dir: String(formData.get("default_backup_dir") || state.meta.settings.default_backup_dir || "./backups"),
    logging: logSettingsFromFormData(formData),
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

function logSettingsFromFormData(formData) {
  const maxSizeMb = numberOrNull(formData.get("log_max_size_mb"));
  const retentionDays = numberOrNull(formData.get("log_retention_days"));
  return {
    max_size_bytes: Math.max(0, Math.round((maxSizeMb ?? 5) * 1024 * 1024)),
    retention_days: retentionDays === null ? null : Math.max(0, Math.round(retentionDays)),
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
    <div class="app-shell ${state.route.name === "manager" || state.route.name === "compression" || state.route.name === "agents" ? "manager-shell" : ""}">
      <nav class="topbar">
        <div class="brand-cluster">
          <a class="brand" href="/" data-nav="/">memorph</a>
          ${renderTopbarContext()}
        </div>
        <div class="top-actions">
          ${state.route.name === "home" ? "" : `<a class="button" href="/" data-nav="/">${state.route.name === "session" ? t("back") : t("openHome")}</a>`}
          <button type="button" data-action="open-workspace-switch">${t("switchWorkspace")}</button>
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
    ${renderToasts()}
  `;
  renderModal();
  bindLocalButtons();
}

function renderTopbarContext() {
  const label = pageTitle();
  if (!label) return "";
  return `
    <span class="topbar-divider"></span>
    <span class="topbar-page">${escapeHtml(label)}</span>`;
}

function pageTitle() {
  switch (state.route.name) {
    case "manager":
      return t("managerTitle");
    case "compression":
      return t("compressionTitle");
    case "agents":
      return t("agentManagementTitle");
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
    case "manager":
      return `<div class="page-scroll manager-page-scroll">${renderManagerPage()}</div>`;
    case "compression":
      return `<div class="page-scroll manager-page-scroll">${renderCompressionPage()}</div>`;
    case "agents":
      return `<div class="page-scroll manager-page-scroll">${renderAgentManagementPage()}</div>`;
    default:
      return `<div class="page-scroll"><div class="empty-state">${t("notFound")}</div></div>`;
  }
}

function renderAgentManagementPage() {
  const selected = currentAgentProvider();
  return `
    <div class="manager-page-layout agent-management-page-layout">
      <section class="section-panel manager-control-panel agent-provider-panel">
        <section class="manager-workspace-summary">
          <div>
            <span class="eyebrow">${t("workspace")}</span>
            <strong>${escapeHtml(workspaceName(state.home.workspace) || t("workspaceEmpty"))}</strong>
            <p>${escapeHtml(state.home.workspace || "—")}</p>
          </div>
        </section>
        ${renderAgentProviderList()}
      </section>
      <section class="section-panel manager-result-panel agent-provider-detail-panel">
        ${selected ? renderAgentProviderDetail(selected) : `<div class="empty-state">${t("noProviders")}</div>`}
      </section>
    </div>`;
}

function currentAgentProvider() {
  return (
    state.agents.providers.find((item) => item.provider_id === state.agents.selectedProvider) ||
    state.agents.providers[0] ||
    null
  );
}

function renderAgentProviderList() {
  if (!state.agents.providers.length) {
    return `<div class="empty-state">${t("noProviders")}</div>`;
  }
  return `
    <div class="manager-list agent-provider-list">
      ${state.agents.providers
        .map((item) => {
          const selected = item.provider_id === state.agents.selectedProvider;
          const installed = agentProviderEnvironment(item).installed;
          const statusLabel = installed ? t("installed") : t("notDetected");
          return `
            <button
              type="button"
              class="agent-provider-item ${selected ? "is-active" : ""}"
              data-action="select-agent-provider"
              data-provider="${escapeAttr(item.provider_id)}"
            >
              <span class="agent-provider-head">
                <strong class="agent-provider-name">${escapeHtml(agentProviderDisplayName(item))}</strong>
                <span class="agent-provider-state ${installed ? "is-installed" : "is-missing"}" title="${escapeAttr(statusLabel)}" aria-label="${escapeAttr(statusLabel)}">
                  ${installed ? "●" : "○"}
                </span>
              </span>
            </button>`;
        })
        .join("")}
    </div>`;
}

function renderAgentProviderDetail(provider) {
  const settings = provider.settings || [];
  const items = settings.filter((setting) => setting.kind === "toggle" || setting.kind === "action");
  const actionItems = items.filter((setting) => setting.kind === "action");
  const environment = agentProviderEnvironment(provider);
  return `
    <div class="agent-provider-detail-scroll">
    <header class="manager-section-head agent-provider-detail-header">
      <div class="stack agent-provider-detail-head">
        <strong>${escapeHtml(agentProviderDisplayName(provider))}</strong>
        <small>${t("agentManagementProviderHint")}</small>
        <div class="pill-row">
          <span class="pill">${escapeHtml(provider.provider_id)}</span>
          <span class="pill">${escapeHtml(environment.installed ? t("installed") : t("notDetected"))}</span>
          <span class="pill">${escapeHtml(environment.install_method || t("unknown"))}</span>
        </div>
      </div>
      <button
        type="button"
        data-action="detect-agent-provider"
        data-provider="${escapeAttr(provider.provider_id)}"
      >${t("detect")}</button>
    </header>
    <div class="stack">
      <div class="section-heading">
        <div>
          <strong>${t("agentManagementEnvironment")}</strong>
          <small>${t("agentManagementEnvironmentHint")}</small>
        </div>
      </div>
      <div class="manager-summary-grid agent-environment-grid">
        ${renderMetaLine(t("agentInstallStatus"), environment.installed ? t("installed") : t("notDetected"))}
        ${renderMetaLine(t("agentInstallMethod"), environment.install_method || t("unknown"))}
      </div>
      <div class="settings-list agent-environment-paths">
        ${renderAgentDetailRow(t("agentExecutablePath"), environment.executable_path || "—", t("agentExecutablePathHint"))}
        ${renderAgentDetailRow(t("agentExecutableDir"), environment.executable_dir || "—", t("agentExecutableDirHint"))}
        ${renderAgentDetailRow(t("agentConfigPath"), environment.config_path || "—", t("agentConfigPathHint"))}
      </div>
    </div>
    <div class="stack">
      <div class="section-heading">
        <div>
          <strong>${t("agentProviderItems")}</strong>
          <small>${t("agentProviderItemsHint")}</small>
        </div>
      </div>
      ${
        items.length
          ? `<div class="settings-list">${items
              .map((setting) =>
                setting.kind === "toggle"
                  ? renderAgentToggleRow(provider, setting)
                  : renderAgentActionRow(provider, setting)
              )
              .join("")}</div>`
          : `<div class="empty-state">${t("agentProviderItemsEmpty")}</div>`
      }
      ${actionItems.map((setting) => renderAgentSettingResult(provider.provider_id, setting.id)).join("")}
    </div>
    </div>`;
}

function renderAgentDetailRow(label, value, hint) {
  return `
    <div class="settings-row agent-detail-row">
      <div class="settings-copy settings-copy-inline">
        <strong>${escapeHtml(label)}</strong>
        <span>${escapeHtml(hint)}</span>
      </div>
      <div class="path-line">${escapeHtml(String(value || "—"))}</div>
    </div>`;
}

function renderAgentToggleRow(provider, setting) {
  return `
    <div class="settings-row">
      <div class="settings-copy settings-copy-inline">
        <strong>${escapeHtml(agentSettingLabel(setting))}</strong>
        <span>${escapeHtml(setting.description || "")}</span>
      </div>
      <label class="settings-check">
        <input
          type="checkbox"
          data-role="agent-setting-toggle"
          data-provider="${escapeAttr(provider.provider_id)}"
          data-setting-id="${escapeAttr(setting.id)}"
          ${setting.value ? "checked" : ""}
        >
        <span>${setting.value ? t("enabled") : t("disabled")}</span>
      </label>
    </div>`;
}

function renderAgentActionRow(provider, setting) {
  const pending = !!state.agents.pendingSettings[`${provider.provider_id}:${setting.id}`];
  const label = agentSettingLabel(setting);
  return `
    <div class="settings-row">
      <div class="settings-copy settings-copy-inline">
        <strong>${escapeHtml(label)}</strong>
        <span>${escapeHtml(setting.description || "")}</span>
      </div>
      <button
        type="button"
        class="invert"
        data-action="run-agent-setting"
        data-provider="${escapeAttr(provider.provider_id)}"
        data-setting-id="${escapeAttr(setting.id)}"
        ${pending ? "disabled" : ""}
      >${escapeHtml(pending ? t("running") : label)}</button>
    </div>`;
}

function renderAgentSettingResult(providerId, settingId) {
  const result = state.agents.settingResults[`${providerId}:${settingId}`];
  if (!result) return "";
  if (settingId === "repair_workspace_sessions") {
    return renderCodexRepairReport(result);
  }
  return `<pre class="code-block">${escapeHtml(JSON.stringify(result, null, 2))}</pre>`;
}

function renderCodexRepairReport(result) {
  const report = result?.type === "codex_workspace_repair" ? result.data : null;
  if (!report) {
    return "";
  }
  const touched = report.touched_sessions || [];
  return `
    <section class="manager-workspace-summary">
      <div class="manager-summary-grid codex-repair-summary-grid">
        ${renderMetaLine(t("workspace"), report.workspace_dir)}
        ${renderMetaLine(t("currentProvider"), report.current_model_provider)}
        ${renderMetaLine(t("scanned"), String(report.scanned_rollouts))}
        ${renderMetaLine(t("workspaceSessions"), String(report.workspace_session_count))}
        ${renderMetaLine(t("hiddenSessions"), String(report.hidden_session_count))}
        ${renderCodexRepairCountLine(report.repaired_session_count, touched.length)}
        ${renderMetaLine(t("reindexedSessions"), String(report.reindexed_session_count))}
        ${renderMetaLine(t("updatedSqliteRows"), String(report.sqlite_rows_updated || 0))}
        ${report.backup_dir ? renderWideMetaLine(t("backupLocation"), report.backup_dir) : ""}
        ${report.pruned_backup_count ? renderMetaLine(t("prunedBackups"), String(report.pruned_backup_count)) : ""}
        ${report.skipped_rollout_files?.length ? renderMetaLine(t("skippedRollouts"), String(report.skipped_rollout_files.length)) : ""}
      </div>
      ${touched.length ? "" : `<div class="empty-state">${t("noRepairNeeded")}</div>`}
    </section>`;
}

function renderCodexRepairCountLine(count, touchedCount) {
  if (!touchedCount) {
    return renderMetaLine(t("repairedSessions"), String(count || 0));
  }
  return `
    <div class="stack">
      <span class="eyebrow">${escapeHtml(t("repairedSessions"))}</span>
      <button type="button" class="meta-link-button" data-action="open-repaired-sessions">
        ${escapeHtml(String(count || 0))}
      </button>
    </div>`;
}

function renderWideMetaLine(label, value) {
  if (value === null || value === undefined || value === "") return "";
  return `<div class="stack meta-line-wide"><span class="eyebrow">${escapeHtml(label)}</span><div class="path-line">${escapeHtml(
    formatValue(value)
  )}</div></div>`;
}

function openRepairedSessionsModal() {
  const result = state.agents.settingResults["codex:repair_workspace_sessions"];
  const report = result?.type === "codex_workspace_repair" ? result.data : null;
  const touched = report?.touched_sessions || [];
  state.modal = {
    kind: "info",
    title: t("repairedSessions"),
    subtitle: t("repairedSessionsHint"),
    body: touched.length
      ? `<div class="session-list repaired-session-list">
          ${touched
            .map(
              (item) => `
                <article class="session-row">
                  <div class="session-row-main">
                    <div class="session-info">
                      <div class="session-title-line">
                        <span class="session-title">${escapeHtml(item.title || item.session_id)}</span>
                        <span class="session-workspace">${escapeHtml(item.current_model_provider || "—")}</span>
                      </div>
                      <div class="session-meta-bar">
                        <span class="session-id-pill">${escapeHtml(item.session_id)}</span>
                        <span class="meta-dot">·</span>
                        <span class="meta-item">${escapeHtml(item.previous_model_provider || "—")} → ${escapeHtml(
                          item.current_model_provider || "—"
                        )}</span>
                        <span class="meta-dot">·</span>
                        <span class="meta-item">${escapeHtml(item.added_to_index ? t("reindexed") : t("providerUpdated"))}</span>
                      </div>
                    </div>
                  </div>
                </article>`
            )
            .join("")}
        </div>`
      : `<div class="empty-state">${t("noRepairNeeded")}</div>`,
  };
  render();
}

function agentSettingLabel(setting) {
  switch (setting.id) {
    case "repair_workspace_sessions":
      return t("repairCurrentWorkspaceSessions");
    case "show_subagents":
      return t("showSubagents");
    default:
      return setting.title || setting.id;
  }
}

function agentProviderDisplayName(provider) {
  switch (provider?.provider_id) {
    case "claude":
      return "Claude";
    case "codex":
      return "Codex";
    case "cursor":
      return "Cursor";
    case "deepseek":
      return "DeepSeek";
    case "kiro":
      return "Kiro";
    case "kimi":
      return "Kimi";
    case "opencode":
      return "OpenCode";
    default:
      return provider?.name || provider?.provider_id || "Unknown";
  }
}

function agentProviderEnvironment(provider) {
  return provider?.environment || {
    installed: provider?.installed || false,
    executable_path: provider?.executable_path || null,
    executable_dir: provider?.executable_dir || null,
    config_path: provider?.config_path || "",
    install_method: provider?.install_method || "",
  };
}

async function updateAgentSetting(providerId, settingId, value) {
  try {
    await api(`/api/v1/providers/${encodeURIComponent(providerId)}/settings/${encodeURIComponent(settingId)}`, {
      method: "PUT",
      body: { value },
    });
    if (providerId === "opencode" && settingId === "show_subagents") {
      state.meta.settings.show_opencode_subagents = value;
    }
    await loadAgentManagement();
    toast(t("saved"), agentSettingLabel({ id: settingId, title: settingId }));
    render();
  } catch (error) {
    toast(t("error"), error.message, true);
  }
}

async function runAgentSetting(providerId, settingId) {
  const key = `${providerId}:${settingId}`;
  if (state.agents.pendingSettings[key]) return;
  state.agents.pendingSettings[key] = true;
  setLoading(true, {
    label: agentSettingLabel({ id: settingId, title: settingId }),
    detail: state.home.workspace ? workspaceName(state.home.workspace) : "",
  });
  try {
    const result = await api(`/api/v1/providers/${encodeURIComponent(providerId)}/settings/${encodeURIComponent(settingId)}`, {
      method: "POST",
      body: {
        workspace: state.home.workspace || null,
      },
    });
    state.agents.settingResults[key] = result;
    await loadAgentManagement();
    if (providerId === "codex" && settingId === "repair_workspace_sessions" && state.home.workspace) {
      updateLoading({ detail: t("refreshingSessions") });
      await loadHome();
    }
    if (!(providerId === "codex" && settingId === "repair_workspace_sessions")) {
      toast(t("done"), agentSettingLabel({ id: settingId, title: settingId }));
    }
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    delete state.agents.pendingSettings[key];
    setLoading(false);
    render();
  }
}

function renderHome() {
  const filteredGroups = sortSessionGroupsByDisplay(filterAndSortGroups(state.home.groups));
  const totalSessions = filteredGroups.reduce((sum, group) => sum + (group.total_sessions || group.sessions.length), 0);
  const shownSessions = filteredGroups.reduce((sum, group) => sum + group.sessions.length, 0);
  return `
    <div class="page-home">
      <section class="home-hero">
        <div class="ascii-banner" style="--ascii-banner-color: ${escapeAttr(state.ui.asciiBannerColor)}"><pre>${escapeHtml(ASCII)}</pre></div>
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
            <a class="session-title session-title-link" href="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}" data-nav="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}">
              ${escapeHtml(item.title || item.session_id)}
            </a>
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
  const sessionMeta = (state.home?.sessions || []).find(
    (s) => s.provider_id === state.route.provider && s.session_id === state.route.sessionId
  );
  const sizeBytes = sessionMeta?.size_bytes;
  return `
    <section class="session-header">
      <div>
        <p class="eyebrow">${escapeHtml(view.provider_name || view.provider_id)}</p>
        <h1>${escapeHtml(view.title || view.session_id || state.route.sessionId)}</h1>
        <div class="meta-line">
          <span>id=<code>${escapeHtml(view.session_id || state.route.sessionId)}</code></span>
          <span>${t("messageCount")}=${view.message_count}</span>
          ${view.last_active_at ? `<span>${t("lastActiveAt")}=${escapeHtml(formatDate(view.last_active_at))}</span>` : ""}
          ${sizeBytes != null ? `<span>${t("size")}=${escapeHtml(formatBytes(sizeBytes))}</span>` : ""}
          ${workspace ? `<span>${t("workspace")}=<code>${escapeHtml(workspace)}</code></span>` : ""}
          ${sharedRef ? `<a class="shared-badge" href="/shared/${sharedRef}" data-nav="/shared/${sharedRef}">${t("activeShared")}</a>` : ""}
        </div>
      </div>
      <div class="session-actions">
        <a class="button" href="/compression" data-nav="/compression">${t("compression")}</a>
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
          ${view.events.length ? view.events.map((event, index) => renderDetailEvent(event, index)).join("") : `<div class="empty-state">${t("noMessages")}</div>`}
        </div>
      </section>
    </div>`;
}

function getBlockLabel(block) {
  switch (block.type) {
    case "text": return "";
    case "thinking": return t("thinking");
    case "tool_call": return `${t("toolUse")}: ${block.name || ""}`.replace(/:\s$/, "");
    case "tool_result": return t("toolResult");
    case "patch": return "Patch";
    case "command": return "Command";
    case "command_result": return "Command Result";
    case "file": return t("file");
    case "image": return t("image");
    case "provider_payload": return block.kind || "payload";
    case "unknown": return t("details");
    default: return "";
  }
}

function getBlockLabels(blocks) {
  return (blocks || []).map(getBlockLabel).filter(Boolean);
}

function countLines(text) {
  if (!text) return 0;
  return text.split('\n').length;
}

function renderDetailEvent(event, index) {
  const blocks = (event.blocks || []).map(renderDetailBlock).join("");
  const role = (event.role || "unknown").replaceAll("_", " ");
  const kind = (event.kind || "unknown").replaceAll("_", " ");
  const blockLabels = getBlockLabels(event.blocks);
  const labelPart = blockLabels.length
    ? ` · ${blockLabels.map((l) => `<span class="msg-block-label">${escapeHtml(l)}</span>`).join(" · ")}`
    : "";
  return `
    <article class="msg-item" data-message-index="${index}" data-role="${escapeAttr(event.role || 'unknown')}">
      <header class="msg-header">
        <span class="msg-header-main">
          <span class="msg-role">${escapeHtml(role)}</span>
          <span>${escapeHtml(kind)}</span>${labelPart}
        </span>
        <span class="msg-header-meta">
          <a href="#" class="text-action" data-action="copy-detail-message" data-message-index="${index}">${t("copy")}</a>
          <a href="#" class="text-action" data-action="delete-detail-message" data-message-index="${index}">${t("remove")}</a>
          <a href="#" class="text-action" data-action="toggle-detail-message" data-message-index="${index}">${t("expand")}</a>
          <span>${escapeHtml(formatDate(event.timestamp))}</span>
        </span>
      </header>
      <div class="msg-body">${blocks || `<p class="muted">${t("noDetails")}</p>`}</div>
    </article>`;
}

function renderDetailBlock(block) {
  switch (block.type) {
    case "text": {
      const lines = countLines(block.text || "");
      const clamp = lines > 3 ? "is-clamped" : "";
      return `<div class="content-block content-text ${clamp}">${markdown(block.text || "")}</div>`;
    }
    case "thinking": {
      const lines = countLines(block.text || "");
      const clamp = lines > 3 ? "is-clamped" : "";
      return `<div class="content-block content-thinking ${clamp}"><p>${escapeHtml(block.text || "")}</p></div>`;
    }
    case "tool_call":
      return `<div class="content-block content-tool"><pre><code>${escapeHtml(
        JSON.stringify(
          { tool_call_id: block.tool_call_id, name: block.name, input: block.input },
          null,
          2
        )
      )}</code></pre></div>`;
    case "tool_result":
      return `<div class="content-block content-tool"><pre><code>${escapeHtml(
        block.content || ""
      )}</code></pre></div>`;
    case "patch":
      return `<div class="content-block content-patch"><pre><code>${escapeHtml(
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
      )}</code></pre></div>`;
    case "command":
      return `<div class="content-block content-command"><pre><code>${escapeHtml(
        JSON.stringify(
          {
            command: block.command,
            argv: block.argv,
            cwd: block.cwd,
          },
          null,
          2
        )
      )}</code></pre></div>`;
    case "command_result":
      return `<div class="content-block content-command-result"><pre><code>${escapeHtml(
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
      )}</code></pre></div>`;
    case "file":
      return `<div class="content-block content-file"><code>${escapeHtml(block.path || "")}</code>${block.content ? `<pre><code>${escapeHtml(block.content)}</code></pre>` : ""}</div>`;
    case "image":
      return `<div class="content-block content-image"><code>${escapeHtml(
        block.path || block.mime_type || ""
      )}</code></div>`;
    case "provider_payload":
      return `<div class="content-block content-provider-payload"><pre><code>${escapeHtml(
        JSON.stringify(block.payload ?? {}, null, 2)
      )}</code></pre></div>`;
    case "unknown":
      return `<div class="content-block content-unknown"><pre><code>${escapeHtml(
        JSON.stringify(block.raw ?? block, null, 2)
      )}</code></pre></div>`;
    default:
      return `<div class="content-block content-unknown"><pre>${escapeHtml(JSON.stringify(block, null, 2))}</pre></div>`;
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

function renderManagerPage() {
  const draft = state.manager.draft || defaultManagerDraft();
  const preview = state.manager.preview;
  const report = state.manager.report;
  return `
    <div class="manager-page-layout">
      <section class="section-panel manager-control-panel">
        ${renderManagerForm(draft)}
      </section>
      <section class="section-panel manager-result-panel">
        ${renderManagerPreview(preview || emptyManagerPreview(), report)}
      </section>
    </div>`;
}

function renderCompressionPage() {
  const archives = state.compression.archives || [];
  const providers = state.compression.providers || [];
  const rows = archives
    .map((archive) => {
      const archiveRef = archive.archive_ref || "";
      return `
        <article class="manager-row">
          <div class="manager-row-head">
            <div class="manager-row-copy">
              <div class="session-title">${escapeHtml(archive.canonical_id || t("compressionArchive"))}</div>
              <div class="manager-meta">
                <span>${escapeHtml(archive.source_provider_id || "—")} → ${escapeHtml(archive.target_provider_id || "—")}</span>
                <span>${t("sourceEvents")}=${escapeHtml(String(archive.source_event_count ?? 0))}</span>
                <span>${t("storedSize")}=${escapeHtml(formatBytes(archive.stored_size_bytes))}</span>
                <span>${t("originalSize")}=${escapeHtml(formatBytes(archive.original_size_bytes))}</span>
                <span>${t("compressionRatio")}=${escapeHtml(formatRatio(archive.compression_ratio))}</span>
                <span>${t("createdAt")}=${escapeHtml(formatDate(archive.created_at))}</span>
              </div>
              <div class="path-line">${escapeHtml(archiveRef)}</div>
            </div>
            <div class="session-actions">
              <button type="button" data-action="open-compression-restore" data-archive-ref="${escapeAttr(archiveRef)}">${t("restore")}</button>
            </div>
          </div>
        </article>`;
    })
    .join("");

  return `
    <div class="manager-page-layout">
      <section class="section-panel manager-control-panel">
        <div class="stack">
          <section class="manager-workspace-summary">
            <div>
              <span class="eyebrow">${t("compression")}</span>
              <strong>${t("compressionTitle")}</strong>
              <p>${t("compressionHint")}</p>
            </div>
          </section>
          ${renderCompressionProviderSupport(providers)}
          <button type="button" data-action="open-compression-expand">${t("expand")}</button>
          <button class="invert" type="button" data-action="refresh-compression">${t("refresh")}</button>
        </div>
      </section>
      <section class="section-panel manager-result-panel">
        <div class="section-heading manager-section-head">
          <div>
            <strong>${t("compressionArchives")}</strong>
            <span>${archives.length}</span>
          </div>
        </div>
        <div class="manager-list">${rows || `<div class="empty-state">${t("emptyCompressionArchives")}</div>`}</div>
      </section>
    </div>`;
}

function renderCompressionProviderSupport(providers) {
  const rows = providers
    .map((provider) => {
      const source = provider.detects_native_source ? t("native") : t("portable");
      const target = provider.native_target_projection ? t("native") : t("portable");
      const defaultProjection = provider.default_projection || "portable";
      return `
        <article class="manager-row">
          <div class="manager-row-head">
            <div class="manager-row-copy">
              <div class="session-title">${escapeHtml(provider.provider_id || "—")}</div>
              <div class="manager-meta">
                <span>${t("source")}=${escapeHtml(source)}</span>
                <span>${t("target")}=${escapeHtml(target)}</span>
                <span>${t("defaultProjection")}=${escapeHtml(defaultProjection)}</span>
              </div>
            </div>
          </div>
        </article>`;
    })
    .join("");
  return `
    <section class="verify-block">
      <span class="block-label">${t("providerCompressionSupport")}</span>
      <div class="manager-list">${rows || `<div class="empty-state">${t("noProviders")}</div>`}</div>
    </section>`;
}

function emptyManagerPreview() {
  return {
    items: [],
    total_count: 0,
    total_size_bytes: 0,
  };
}

function renderManagerForm(managerDraft) {
  const providerChecks = getOrderedProviders()
    .map((item) => {
      const checked = managerDraft.providers.includes(item.id);
      return `
        <label class="agent-provider-item manager-provider-item ${checked ? "is-active" : ""}">
          <input data-role="manager-provider-toggle" type="checkbox" name="manager_provider" value="${escapeAttr(item.id)}" ${checked ? "checked" : ""}>
          <span class="agent-provider-head">
            <strong class="agent-provider-name">${escapeHtml(item.name)}</strong>
            <span class="agent-provider-state ${checked ? "is-installed" : "is-missing"}" aria-hidden="true">${checked ? "●" : "○"}</span>
          </span>
        </label>`;
    })
    .join("");
  return `
    <div class="manager-control-content">
      <section class="manager-workspace-summary">
        <div>
          <span class="eyebrow">${t("workspace")}</span>
          <strong>${escapeHtml(workspaceName(state.home.workspace) || t("workspaceEmpty"))}</strong>
          <p>${escapeHtml(state.home.workspace || "—")}</p>
        </div>
      </section>
      <section class="manager-control-bottom">
        <section class="stack">
          <div class="section-heading">
            <div>
              <strong>${t("providers")}</strong>
            </div>
          </div>
          <div class="manager-list agent-provider-list manager-provider-list">${providerChecks}</div>
        </section>
      </section>
    </div>`;
}

function openManagerFilterModal() {
  const draft = state.manager.draft || defaultManagerDraft();
  const providerInputs = draft.providers
    .map((provider) => `<input type="hidden" name="providers" value="${escapeAttr(provider)}">`)
    .join("");
  state.modal = {
    kind: "form",
    title: t("preview"),
    submit: "preview-manager",
    body: `
      ${providerInputs}
      <div class="stack">
        <section class="manager-filter-grid">
          <div class="filter-card filter-card-wide">
            <span class="filter-label">${t("olderThanDays")}</span>
            <div class="unit-field">
              <input type="text" inputmode="decimal" name="older_than_days" placeholder="30" value="${escapeAttr(
                draft.older_than_days
              )}">
              <select name="older_than_unit">
                ${unitOption("minutes", t("minutes"), draft.older_than_unit)}
                ${unitOption("hours", t("hours"), draft.older_than_unit)}
                ${unitOption("days", t("days"), draft.older_than_unit)}
                ${unitOption("weeks", t("weeks"), draft.older_than_unit)}
                ${unitOption("months", t("months"), draft.older_than_unit)}
              </select>
            </div>
          </div>
          <div class="filter-card">
            <span class="filter-label">${t("storageGreaterThan")}</span>
            <div class="size-bound-field">
              <input type="text" inputmode="decimal" name="size_min_value" placeholder="1" value="${escapeAttr(
                draft.size_min_value
              )}">
              <select name="size_min_unit">
                ${unitOption("kb", "KB", draft.size_min_unit)}
                ${unitOption("mb", "MB", draft.size_min_unit)}
                ${unitOption("gb", "GB", draft.size_min_unit)}
              </select>
            </div>
          </div>
          <div class="filter-card">
            <span class="filter-label">${t("storageLessThan")}</span>
            <div class="size-bound-field">
              <input type="text" inputmode="decimal" name="size_max_value" placeholder="10" value="${escapeAttr(
                draft.size_max_value
              )}">
              <select name="size_max_unit">
                ${unitOption("kb", "KB", draft.size_max_unit)}
                ${unitOption("mb", "MB", draft.size_max_unit)}
                ${unitOption("gb", "GB", draft.size_max_unit)}
              </select>
            </div>
          </div>
        </section>
        <section class="manager-keyword-grid">
          <label class="field compact-keyword-field">
            <span>${t("managerMaxResults")}</span>
            <input type="text" inputmode="numeric" name="max_results" value="${escapeAttr(draft.max_results || "")}" placeholder="10">
          </label>
          <label class="field compact-keyword-field">
            <span>${t("managerSortOrder")}</span>
            <select name="sort_order">
              ${unitOption("recent", t("managerSortRecentDesc"), draft.sort_order || "recent")}
              ${unitOption("size", t("managerSortSizeDesc"), draft.sort_order || "recent")}
            </select>
          </label>
        </section>
        <section class="manager-keyword-grid">
          <label class="field compact-keyword-field">
            <span>${t("titleContains")}</span>
            <input name="title_contains" value="${escapeAttr(draft.title_contains || "")}" placeholder="${escapeAttr(t("keywordPlaceholder"))}">
          </label>
          <label class="field compact-keyword-field">
            <span>${t("titleExcludes")}</span>
            <input name="title_excludes" value="${escapeAttr(draft.title_excludes || "")}" placeholder="${escapeAttr(t("keywordPlaceholder"))}">
          </label>
        </section>
        <p class="muted">${t("managerFilterFutureHint")}</p>
      </div>`,
    submitLabel: t("preview"),
  };
  render();
}

function unitOption(value, label, selected) {
  return `<option value="${escapeAttr(value)}" ${selected === value ? "selected" : ""}>${escapeHtml(label)}</option>`;
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
  const actionsInHead = formOpen && modal.actionsInHead;
  modalRoot.innerHTML = `
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
                      <button type="submit" class="${modal.submitClass || "invert"}">${escapeHtml(modal.submitLabel || t("save"))}</button>
                    </div>`
              }
            </form>`
            : modal.body
        }
      </div>
    </div>`;
}

function renderLoading() {
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

function renderToasts() {
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

function closeToast(index) {
  if (!Number.isInteger(index)) return;
  state.toasts = state.toasts.filter((_, itemIndex) => itemIndex !== index);
  render();
}

function closeModal() {
  state.modal = null;
  render();
}

async function copyDetailMessage(index) {
  const event = state.session?.view?.events?.[index];
  if (!event) return;
  const text = detailEventToText(event);
  try {
    await navigator.clipboard.writeText(text);
    toast(t("copied"), t("copy"));
  } catch (error) {
    toast(t("error"), template(t("clipboardCopyFailed"), { error: error.message || String(error) }), true);
  }
}

function toggleDetailMessage(trigger) {
  const item = trigger?.closest?.(".msg-item");
  if (!item) return;
  item.classList.toggle("is-expanded");
}

function detailEventToText(event) {
  return (event.blocks || [])
    .map((block) => {
      if (block.type === "text") return block.text || "";
      if (block.type === "thinking") return block.text || "";
      if (block.type === "tool_result") return block.content || "";
      if (block.type === "file") return [block.path, block.content].filter(Boolean).join("\n");
      return JSON.stringify(block.raw ?? block.payload ?? block, null, 2);
    })
    .filter(Boolean)
    .join("\n\n");
}

function githubIcon() {
  return `<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.6 7.6 0 0 1 8 3.86c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>`;
}

function isExternalHttpUrl(url) {
  if (!url) return false;
  try {
    const parsed = new URL(url, window.location.href);
    return /^https?:$/.test(parsed.protocol) && parsed.origin !== window.location.origin;
  } catch {
    return false;
  }
}

async function openExternalUrl(url) {
  if (!url) return;
  try {
    await api("/api/v1/system/open-external", {
      method: "POST",
      body: { url },
    });
    return;
  } catch (_error) {
    // Fall through to browser-side open as a best-effort fallback.
  }
  const tauriOpener = window.__TAURI__?.opener?.openUrl;
  if (typeof tauriOpener === "function") {
    await tauriOpener(url);
    return;
  }
  const opened = window.open(url, "_blank", "noopener,noreferrer");
  if (!opened) {
    window.location.href = url;
  }
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
  if (pathname === "/agents" || pathname === "/tools") return { name: "agents" };
  if (pathname === "/manager") return { name: "manager" };
  if (pathname === "/compression") return { name: "compression" };
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

function formatRatio(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return "—";
  return `${(number * 100).toFixed(1)}%`;
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

function setLoading(active, info = null) {
  state.loading += active ? 1 : -1;
  if (state.loading < 0) state.loading = 0;
  if (active && info) state.loadingInfo = info;
  if (!active && state.loading === 0) state.loadingInfo = null;
  render();
}

function updateLoading(info) {
  state.loadingInfo = { ...(state.loadingInfo || {}), ...info };
  render();
}
