import { ABOUT_LINKS, ASCII, DETAIL_EVENT_PAGE_SIZE } from "./app/constants.js";
import { createAgentsSettingsModule } from "./app/agents_settings.js";
import {
  closeModal as closeChromeModal,
  closeToast as closeChromeToast,
  fatal as renderFatal,
  renderAppShell,
  renderLoadingMarkup,
  renderModalMarkup,
  renderToastsMarkup,
  renderTopbarContext as renderTopbarContextMarkup,
  setLoading as setChromeLoading,
  toast as showToast,
  updateLoading as updateChromeLoading,
} from "./app/chrome.js";
import { createHomeModule } from "./app/home.js";
import { createHooksCenterModule } from "./app/hooks_center.js";
import { createManagerCompressionModule } from "./app/manager_compression.js";
import { createModalModule } from "./app/modals.js";
import { createSessionSyncModule } from "./app/session_sync.js";
import { createFormatHelpers } from "./app/helpers.js";
import { api } from "./app/http.js";
import { createI18n } from "./app/i18n.js";
import { createNavigation } from "./app/navigation.js";
import { createProviders } from "./app/providers.js";
import { parseRoute, routeBackTarget, routeScrollClass } from "./app/router.js";
import { createState, randomAsciiBannerColor } from "./app/state.js";

const state = createState();
const providers = createProviders(state);
const HOME_HERO_MODE_STORAGE_KEY = "memorph.homeHeroMode";
const CATALOG_CLIENT_CACHE_TTL_MS = 60_000;
const catalogClientCache = new Map();
restoreHomeHeroMode();
const { lang, t, loadI18n, setDocumentLanguage } = createI18n(
  () => state.meta?.settings?.language
);
const {
  shortId,
  markdown,
  formatContent,
  formatDate,
  formatValue,
  formatBytes,
  formatRatio,
  workspaceName,
  emptyToNull,
  numberOrNull,
  escapeHtml,
  escapeAttr,
} = createFormatHelpers(lang);
const {
  findSyncRef,
  getDefaultSwitchTarget,
  getFoldedProviders,
  getOrderedProviders,
  providerOptions: homeProviderOptions,
  renderHome,
  renderWorkspacePicker,
  scheduleHomeProviderLayout,
} = createHomeModule({
  state,
  providers,
  ascii: ASCII,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  workspaceName,
  render: () => render(),
});
const renderMetaLine = (label, value) => {
  if (value === null || value === undefined || value === "") return "";
  return `<div class="stack"><span class="eyebrow">${escapeHtml(label)}</span><div class="path-line">${escapeHtml(
    formatValue(value)
  )}</div></div>`;
};
const {
  agentSettingLabel,
  openRepairedSessionsModal,
  openSettingsModal,
  renderAgentDetailRow,
  renderAgentManagementPage,
} = createAgentsSettingsModule({
  state,
  providers,
  aboutLinks: ABOUT_LINKS,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatValue,
  workspaceName,
  renderMetaLine,
  render: () => render(),
});
const {
  detailEventToText,
  renderSessionDetail,
  renderSyncDetail,
  renderSyncList,
} = createSessionSyncModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  markdown,
  formatContent,
  renderAgentDetailRow,
  renderMetaLine,
  findSyncRef,
});
const {
  renderCompressionPage,
  renderCompressionArchiveDetail,
  renderManagerPage,
  unitOption,
  updateManagerSelectionStats,
  selectedManagerWorkspaceItems,
} = createManagerCompressionModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  formatRatio,
  markdown,
  formatContent,
  workspaceName,
  getOrderedProviders,
  renderMetaLine,
  selectedManagerItems: () => selectedManagerItems(),
  defaultManagerDraft,
});
const { renderHooksCenterPage } = createHooksCenterModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatValue,
  renderMetaLine,
  workspaceName,
});

const appEl = document.getElementById("app");
const modalRoot = document.getElementById("modal-root");
const navigation = createNavigation({
  parseRoute,
  getRoute: () => state.route,
  setRoute: (route) => {
    state.route = route;
  },
  fallbackPath: routeBackTarget,
  onBeforeNavigation: () => {
    state.modal = null;
  },
  onHomeRoute: () => {
    state.ui.asciiBannerColor = randomAsciiBannerColor();
  },
  loadRoute: () => loadRoute(),
  readPageState: (route) => readPageState(route),
  applyPageState: (route, pageState) => applyPageState(route, pageState),
});

navigation.init();

window.addEventListener("resize", () => {
  scheduleHomeProviderLayout();
});

document.addEventListener("click", (event) => {
  const nav = event.target.closest("[data-nav]");
  if (nav?.dataset.nav) {
    event.preventDefault();
    navigate(nav.dataset.nav);
    return;
  }

  const rowAction = event.target.closest(".manager-row-actions");
  if (rowAction && !event.target.closest("[data-action], [data-nav]")) {
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

document.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") return;
  const action = event.target.closest('[role="button"][data-action]');
  if (!action) return;
  event.preventDefault();
  void handleAction(action.dataset.action, action.dataset, action);
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
    autoCollapseHomeHero();
    render();
  }
});

void bootstrap();

function restoreHomeHeroMode() {
  try {
    const stored = window.localStorage?.getItem(HOME_HERO_MODE_STORAGE_KEY);
    if (["auto", "expanded", "collapsed"].includes(stored)) {
      state.ui.homeHeroMode = stored;
    }
  } catch {
    state.ui.homeHeroMode = "auto";
  }
}

function setHomeHeroMode(mode, refreshBanner = false) {
  if (!["auto", "expanded", "collapsed"].includes(mode)) return;
  state.ui.homeHeroMode = mode;
  state.ui.homeHeroTransientCollapsed = false;
  if (refreshBanner) {
    state.ui.asciiBannerColor = randomAsciiBannerColor();
  }
  try {
    window.localStorage?.setItem(HOME_HERO_MODE_STORAGE_KEY, mode);
  } catch {
    // Ignore unavailable local preferences storage.
  }
  render();
}

function autoCollapseHomeHero() {
  if (state.ui.homeHeroMode !== "auto") return;
  state.ui.homeHeroTransientCollapsed = true;
}

async function refreshCatalog(workspace = state.home.workspace, options = {}) {
  const key = workspace || "";
  const cached = catalogClientCache.get(key);
  if (!options.force && cached && Date.now() - cached.createdAt < CATALOG_CLIENT_CACHE_TTL_MS) {
    state.catalog = cached.catalog;
    return state.catalog;
  }
  const query = workspace ? `?workspace=${encodeURIComponent(workspace)}` : "";
  state.catalog = await api(`/api/v1/providers/catalog${query}`);
  catalogClientCache.set(key, { catalog: state.catalog, createdAt: Date.now() });
  return state.catalog;
}

function invalidateCatalogClientCache(workspace = null) {
  if (workspace === null) catalogClientCache.clear();
  else catalogClientCache.delete(workspace || "");
}

function homeProviderCandidates() {
  return providers
    .all()
    .filter((item) => !item.hidden_state?.global)
    .filter((item) => providers.hasFilter(item, "is_installed"))
    .map((item) => item.provider_id);
}

function hasWorkspaceProviderSelection(workspace = state.home.workspace) {
  if (!workspace) return false;
  return (state.meta?.workspaces || []).some((item) => item.path === workspace && Array.isArray(item.providers) && item.providers.length > 0);
}

async function resolveHomeProviders(workspace = state.home.workspace) {
  const candidates = homeProviderCandidates();
  if (!workspace || !candidates.length) return candidates;
  if (!hasWorkspaceProviderSelection(workspace)) return candidates;

  const saved = await api(`/api/v1/workspaces/providers?workspace=${encodeURIComponent(workspace)}`);
  const selected = saved.filter((id) => candidates.includes(id));
  return selected.length ? selected : candidates;
}

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
    await refreshCatalog(state.home.workspace);
    state.home.providers = await resolveHomeProviders(state.home.workspace);
    await loadRoute();
  } catch (error) {
    fatal(error);
  } finally {
    setLoading(false);
    navigation.restorePageState();
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
      const detailParams = new URLSearchParams({
        event_offset: "0",
        event_limit: String(DETAIL_EVENT_PAGE_SIZE),
      });
      const statsUrl = `/api/v1/sessions/${encodeURIComponent(route.provider)}/${encodeURIComponent(route.sessionId)}/stats`;
      const [detail, stats] = await Promise.all([
        api(
          `/api/v1/sessions/${encodeURIComponent(route.provider)}/${encodeURIComponent(route.sessionId)}?${detailParams.toString()}`
        ),
        api(statsUrl).catch(() => null),
      ]);
      state.session = {
        ...detail,
        hook_runtime_sessions: detail.hook_runtime_sessions || [],
        stats: stats || null,
      };
      const detailWorkspace = detail.view?.workspace_dir;
      if (detailWorkspace && detailWorkspace !== state.home.workspace) {
        state.home.workspace = detailWorkspace;
        await refreshCatalog(detailWorkspace);
        state.home.providers = await resolveHomeProviders(detailWorkspace);
      }
      state.home.syncGroups = await api("/api/v1/sync/status");
    } else if (route.name === "sync-list") {
      state.home.syncGroups = await api("/api/v1/sync/status");
    } else if (route.name === "sync-detail") {
      state.home.syncGroups = await api("/api/v1/sync/status");
      state.syncDetail = await api(
        `/api/v1/sync/status?group_id=${encodeURIComponent(route.groupId)}`
      );
    } else if (route.name === "manager") {
      state.home.syncGroups = await api("/api/v1/sync/status");
      state.manager.viewMode = route.view === "workspaces" ? "workspaces" : "sessions";
      if (route.provider || route.workspace) {
        await loadManagerRoutePreview(route);
      } else {
        if (!state.manager.quickPreview) {
          await loadDefaultManagerPreview();
        }
        // If landing directly on workspaces view but quick workspace data not loaded yet, fetch it
        if (state.manager.viewMode === "workspaces" && !state.manager.quickWorkspacePreview && state.manager.isDefaultPreview) {
          const draft = state.manager.draft || defaultManagerDraft();
          if (draft.providers.length) {
            const providerQuery = draft.providers.join(",");
            api(`/api/v1/manager/quick-workspaces?providers=${encodeURIComponent(providerQuery)}`)
              .then((quickWorkspacePreview) => {
                state.manager.quickWorkspacePreview = quickWorkspacePreview;
                render();
              })
              .catch((error) => toast(t("error"), error.message, true));
          }
        }
      }
    } else if (route.name === "compression") {
      const workspaceQuery = state.home.workspace
        ? `?workspace=${encodeURIComponent(state.home.workspace)}`
        : "";
      const [archives, providers] = await Promise.all([
        api(`/api/v1/compression/archives${workspaceQuery}`),
        api("/api/v1/compression/providers"),
      ]);
      state.compression.archives = archives;
      state.compression.providers = providers;
    } else if (route.name === "hooks") {
      await loadHooksCenter();
    } else if (route.name === "agents") {
      await loadAgentManagement();
    }
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    setLoading(false);
  }
}

async function loadHome() {
  if (!state.home.workspace) {
    state.home.groups = [];
    state.home.syncGroups = await api("/api/v1/sync/status");
    return;
  }
  const params = new URLSearchParams({
    workspace: state.home.workspace,
    all: "false",
    provider: state.home.providers.join(","),
    details: "false",
    limit: String(Math.max(1, Number(state.home.visible || 12))),
    sort: state.home.sort,
    hook_filter: state.home.hookFilter,
  });
  state.home.groups = await api(`/api/v1/sessions?${params.toString()}`);
  state.home.syncGroups = await api("/api/v1/sync/status");
}

async function refreshHomeSessions(options = {}) {
  if (state.route.name !== "home") return;
  const label = options.label || t("refreshingSessions");
  setLoading(true, { label, detail: workspaceName(state.home.workspace) });
  try {
    await loadHome();
    render();
  } finally {
    setLoading(false);
  }
}

async function loadCompressionArchiveDetail(archiveRef) {
  if (!archiveRef) return;
  setLoading(true, { label: t("loading"), detail: archiveRef });
  try {
    const archive = await api(
      `/api/v1/compression/archive?archive_ref=${encodeURIComponent(archiveRef)}`
    );
    state.compression.selectedArchive = archive;
    state.modal = {
      kind: "custom",
      title: t("compressionArchiveDetail"),
      body: renderCompressionArchiveDetail(archive),
    };
    render();
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    setLoading(false);
  }
}

async function loadMoreSessionEvents() {
  const current = state.session;
  const view = current?.view;
  if (!view || !current.has_more_events) return;

  const params = new URLSearchParams({
    event_offset: String(view.events.length),
    event_limit: String(DETAIL_EVENT_PAGE_SIZE),
  });
  setLoading(true);
  try {
    const next = await api(
      `/api/v1/sessions/${encodeURIComponent(state.route.provider)}/${encodeURIComponent(state.route.sessionId)}?${params.toString()}`
    );
    state.session = {
      ...current,
      ...next,
      hook_runtime_sessions: current.hook_runtime_sessions || [],
      stats: current.stats || null,
      view: {
        ...view,
        ...next.view,
        events: [...view.events, ...(next.view?.events || [])],
      },
    };
  } finally {
    setLoading(false);
    render();
  }
}

async function loadAgentManagement() {
  const [payload, runtimeSessions] = await Promise.all([
    api("/api/v1/agents/summary"),
    api("/api/v1/hooks/runtime-sessions").catch(() => []),
  ]);
  state.agents.providers = payload.providers || [];
  state.agents.hookRuntimeSessions = runtimeSessions || [];
  if (
    !state.agents.selectedProvider ||
    !state.agents.providers.some((item) => item.provider_id === state.agents.selectedProvider)
  ) {
    state.agents.selectedProvider = state.agents.providers[0]?.provider_id || "";
  }
  loadSelectedAgentProviderDetail();
}

async function loadAgentProviderDetail(providerId, options = {}) {
  if (!providerId) return null;
  if (!options.refresh && state.agents.providerDetails[providerId]) {
    return state.agents.providerDetails[providerId];
  }
  state.agents.providerDetailLoading[providerId] = true;
  render();
  try {
    const provider = await api(`/api/v1/agents/${encodeURIComponent(providerId)}`);
    state.agents.providerDetails[provider.provider_id] = provider;
    mergeAgentProvider(provider);
    return provider;
  } finally {
    delete state.agents.providerDetailLoading[providerId];
    render();
  }
}

function loadSelectedAgentProviderDetail() {
  const providerId = state.agents.selectedProvider;
  if (!providerId || state.agents.providerDetails[providerId] || state.agents.providerDetailLoading[providerId]) return;
  loadAgentProviderDetail(providerId).catch((error) => toast(t("error"), error.message, true));
}

function mergeAgentProvider(provider) {
  const index = state.agents.providers.findIndex((item) => item.provider_id === provider.provider_id);
  if (index >= 0) {
    state.agents.providers[index] = { ...state.agents.providers[index], ...provider };
  } else {
    state.agents.providers.push(provider);
  }
}

async function loadHooksCenter() {
  const diagnosisParams = new URLSearchParams({
    hook_filter: state.hooks.diagnosisFilter || "attention",
    limit: "8",
  });
  const [overview, sessionDiagnosis] = await Promise.all([
    api("/api/v1/hooks/overview"),
    api(`/api/v1/hooks/session-diagnosis?${diagnosisParams.toString()}`).catch(() => []),
  ]);
  state.hooks.overview = overview;
  state.hooks.sessionDiagnosis = sessionDiagnosis || [];
  state.agents.providers = state.hooks.overview.providers || [];
  if (
    !state.hooks.selectedProvider ||
    !state.agents.providers.some((provider) => provider.provider_id === state.hooks.selectedProvider)
  ) {
    state.hooks.selectedProvider = state.agents.providers[0]?.provider_id || "";
  }
  if (state.hooks.selectedProvider) {
    await loadHookProviderOverview(state.hooks.selectedProvider);
  } else {
    state.hooks.providerDetail = null;
  }
}

async function loadHookProviderOverview(providerId) {
  state.hooks.providerDetail = await api(
    `/api/v1/hooks/providers/${encodeURIComponent(providerId)}/overview`
  );
}

async function loadRuntimeSessionsForDetail(view) {
  const provider = view?.provider_id || state.route.provider;
  const sessionId = view?.session_id || state.route.sessionId;
  const exactParams = new URLSearchParams({ provider, session_id: sessionId });
  try {
    let sessions = await api(`/api/v1/hooks/runtime-sessions?${exactParams.toString()}`);
    if (sessions.length) return sessions;

    const providerParams = new URLSearchParams({ provider });
    sessions = await api(`/api/v1/hooks/runtime-sessions?${providerParams.toString()}`);
    const workspace = view?.workspace_dir || "";
    return sessions.filter((session) => runtimeSessionMatchesDetail(session, provider, sessionId, workspace));
  } catch {
    return [];
  }
}

function runtimeSessionMatchesDetail(session, provider, sessionId, workspace) {
  if (!session || session.provider !== provider) return false;
  const providerSessionId = session.provider_session_id || "";
  if (
    providerSessionId === sessionId ||
    providerSessionId === `${provider}-${sessionId}` ||
    providerSessionId.endsWith(`-${sessionId}`)
  ) {
    return true;
  }
  const cwd = session.cwd || "";
  return !!workspace && !!cwd && normalizePathForCompare(cwd) === normalizePathForCompare(workspace);
}

function normalizePathForCompare(value) {
  return String(value || "").replace(/\\/g, "/").replace(/\/+$/, "");
}

async function detectAgentProvider(providerId) {
  if (!providerId) return;
  const provider = await api(`/api/v1/agents/${encodeURIComponent(providerId)}/detect`, {
    method: "POST",
  });
  state.agents.providerDetails[provider.provider_id] = provider;
  mergeAgentProvider(provider);
  state.agents.selectedProvider = provider.provider_id;
  invalidateCatalogClientCache();
  render();
}

async function loadHookDiagnostics() {
  state.agents.hookDiagnostics = await api("/api/v1/hooks/diagnostics?event_limit=25&error_limit=25");
  render();
}

async function runHookDoctor(repair = false) {
  const report = await api("/api/v1/hooks/doctor", {
    method: "POST",
    body: { repair },
  });
  state.agents.hookDoctorReport = report;
  if (state.route.name === "hooks") {
    await loadHooksCenter();
  } else {
    await loadHookDiagnostics();
  }
  toast(t("saved"), repair ? "Hook doctor repair completed" : "Hook doctor check completed");
  render();
}

async function cleanupHookRuntimeSessions() {
  const report = await api("/api/v1/hooks/cleanup", { method: "POST" });
  state.agents.hookCleanupReport = report;
  const [runtimeSessions, diagnostics] = await Promise.all([
    api("/api/v1/hooks/runtime-sessions").catch(() => []),
    api("/api/v1/hooks/diagnostics?event_limit=25&error_limit=25").catch(() => state.agents.hookDiagnostics),
  ]);
  state.agents.hookRuntimeSessions = runtimeSessions || [];
  state.agents.hookDiagnostics = diagnostics;
  if (state.route.name === "hooks") {
    await loadHooksCenter();
  }
  toast(t("saved"), `Hook cleanup: idle ${report.idle || 0}, orphaned ${report.orphaned || 0}`);
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
  if (!new Set(["general", "display", "order", "hook", "config", "about"]).has(section)) return;
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

function navigate(path) {
  navigation.navigate(path);
}

function goBack() {
  navigation.goBack();
}

function replacePath(path) {
  navigation.replacePath(path);
}

function replaceNavigate(path) {
  navigation.replaceNavigate(path);
}

function pageStateContainer() {
  return document.querySelector(".page-scroll") || document.querySelector(".page-home");
}

function readPageState(route) {
  const container = pageStateContainer();
  return {
    routeName: route?.name || "home",
    scrollTop: container?.scrollTop || 0,
    homeHeroTransientCollapsed: state.ui.homeHeroTransientCollapsed,
  };
}

function applyPageState(route, pageState) {
  if (!pageState || pageState.routeName !== route?.name) return;
  const container = pageStateContainer();
  if (container) {
    container.scrollTop = Number(pageState.scrollTop || 0);
  }
  if (route?.name === "home") {
    state.ui.homeHeroTransientCollapsed = !!pageState.homeHeroTransientCollapsed;
  }
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

async function setWorkspace(workspace, options = {}) {
  setLoading(true, { label: t("loadingWorkspace"), detail: workspaceName(workspace) });
  state.home.workspace = workspace;
  try {
    const reloadManager = options.reloadManager ?? state.route.name === "manager";
    const loadHomeSessions = options.loadHome ?? true;
    if (reloadManager) {
      state.manager = {
        draft: null,
        preview: null,
        workspacePreview: null,
        quickPreview: null,
        quickWorkspacePreview: null,
        isDefaultPreview: true,
        stats: null,
        statsLoading: false,
        statsRequestId: "",
        report: null,
        pendingItems: [],
        viewMode: state.manager.viewMode || "sessions",
        selectionMode: false,
        selectedItems: new Set(),
        selectedWorkspaceItems: new Set(),
      };
    }
    await refreshCatalog(workspace);
    if (!workspace) {
      state.home.providers = [];
      if (loadHomeSessions) await loadHome();
      await refreshWorkspaceMeta();
      state.meta = { ...state.meta, ...(await api("/api/v1/meta")) };
      if (reloadManager) {
        await loadDefaultManagerPreview();
      } else {
        render();
      }
      return;
    }
    state.home.providers = await resolveHomeProviders(workspace);
    if (loadHomeSessions) await loadHome();
    if (loadHomeSessions && state.route.name === "agents") {
      await loadAgentManagement();
    }
    if (loadHomeSessions && state.route.name === "hooks") {
      await loadHooksCenter();
    }
    await refreshWorkspaceMeta();
    state.meta = { ...state.meta, ...(await api("/api/v1/meta")) };
    if (reloadManager) {
      await loadDefaultManagerPreview();
    } else {
      render();
    }
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
  const nextDraft = { ...draft, providers: [...next] };
  state.manager.draft = nextDraft;

  // Optimistic UI: update the provider row immediately
  const item = trigger?.closest?.(".manager-provider-item");
  if (item) {
    item.classList.toggle("is-active", checked);
    const stateMarker = item.querySelector(".agent-provider-state");
    if (stateMarker) {
      stateMarker.classList.toggle("is-installed", checked);
      stateMarker.classList.toggle("is-missing", !checked);
      stateMarker.textContent = checked ? "●" : "○";
    }
  }

  if (!nextDraft.providers.length) {
    const emptyPreview = emptyManagerPreview();
    state.manager = {
      ...state.manager,
      draft: nextDraft,
      quickPreview: emptyPreview,
      quickWorkspacePreview: null,
      preview: null,
      workspacePreview: null,
      stats: null,
      statsLoading: false,
      statsRequestId: "",
      report: null,
      pendingItems: [],
      selectionMode: false,
      selectedItems: new Set(),
      selectedWorkspaceItems: new Set(),
    };
    render();
    return;
  }

  const isDefault = state.manager.isDefaultPreview;
  setLoading(true, { label: t("managerPreview"), detail: t("scanningSessions") });
  try {
    if (isDefault) {
      // Quick view mode: refresh quick-preview (and invalidate quick-workspaces cache)
      const providerQuery = nextDraft.providers.join(",");
      const quickPreview = await api(`/api/v1/manager/quick-preview?providers=${encodeURIComponent(providerQuery)}`);
      state.manager = {
        ...state.manager,
        draft: nextDraft,
        quickPreview,
        quickWorkspacePreview: null, // invalidate; will lazy-reload if user switches view
        preview: null,
        workspacePreview: null,
        stats: null,
        statsLoading: true,
        statsRequestId: "",
        report: null,
        pendingItems: [],
        selectedItems: new Set(),
        selectedWorkspaceItems: new Set(),
      };
    } else {
      // Filtered view mode: re-run full scan with updated providers
      const refreshResult = await refreshManagerData(nextDraft);
      if (refreshResult.preview) {
        applyManagerDraftFilters(refreshResult.preview, nextDraft);
        refreshResult.preview.output_dir = "";
      }
      state.manager = {
        ...state.manager,
        draft: nextDraft,
        preview: state.manager.viewMode === "sessions" ? refreshResult.preview : null,
        workspacePreview: state.manager.viewMode === "workspaces" ? refreshResult.workspacePreview : null,
        stats: null,
        statsLoading: true,
        statsRequestId: "",
        report: null,
        pendingItems: [],
        selectedItems: new Set(),
        selectedWorkspaceItems: new Set(),
      };
    }
    render();
    queueManagerStatsLoad(nextDraft);
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    setLoading(false);
  }
}

async function persistProvidersAndReload() {
  if (!state.home.workspace) return;
  try {
    const candidateIds = homeProviderCandidates();
    const hiddenWorkspace = candidateIds.filter(
      (id) => !state.home.providers.includes(id)
    );
    await api("/api/v1/workspaces/providers", {
      method: "PUT",
      body: {
        workspace: state.home.workspace,
        providers: state.home.providers,
      },
    });
    await api("/api/v1/providers/catalog", {
      method: "PUT",
      body: {
        sort_order: { global: [], workspace: [] },
        hidden_state: { global: [], workspace: hiddenWorkspace },
        workspace: state.home.workspace,
      },
    });
    invalidateCatalogClientCache(state.home.workspace);
    await refreshCatalog(state.home.workspace, { force: true });
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
      state.agents.providerDetails = {};
      await loadAgentManagement();
      render();
      break;
    case "refresh-hooks":
      await loadHooksCenter();
      render();
      break;
    case "select-hook-provider":
      state.hooks.selectedProvider = data.provider || "";
      if (state.hooks.selectedProvider) {
        await loadHookProviderOverview(state.hooks.selectedProvider);
      } else {
        state.hooks.providerDetail = null;
      }
      render();
      break;
    case "set-hook-diagnosis-filter":
      state.hooks.diagnosisFilter = data.filter || "attention";
      await loadHooksCenter();
      render();
      break;
    case "load-hook-diagnostics":
      await loadHookDiagnostics();
      break;
    case "run-hook-doctor":
      await runHookDoctor(data.repair === "true" || data.repair === true);
      break;
    case "cleanup-hook-runtime":
      await cleanupHookRuntimeSessions();
      break;
    case "select-agent-provider":
      state.agents.selectedProvider = data.provider || "";
      loadSelectedAgentProviderDetail();
      render();
      break;
    case "run-agent-setting":
      await runAgentSetting(data.provider, data.settingId);
      break;
    case "run-hook-operation-now":
      await runHookOperation(data.provider, data.settingId);
      break;
    case "open-hook-operation-confirm":
      openHookOperationConfirm(data.provider, data.settingId);
      break;
    case "load-more-session-events":
      await loadMoreSessionEvents();
      break;
    case "open-repaired-sessions":
      openRepairedSessionsModal();
      break;
    case "open-compression":
      navigate("/compression");
      break;
    case "compress-session":
      openCompressSessionModal(data.provider || "", data.sessionId || "");
      break;
    case "open-compression-restore":
      openCompressionRestoreModal(data.archiveRef || "");
      break;
    case "open-compression-detail":
      await loadCompressionArchiveDetail(data.archiveRef || "");
      break;
    case "open-compression-expand":
      openCompressionExpandModal();
      break;
    case "refresh-compression":
      {
        const workspaceQuery = state.home.workspace
          ? `?workspace=${encodeURIComponent(state.home.workspace)}`
          : "";
        const [archives, providers] = await Promise.all([
          api(`/api/v1/compression/archives${workspaceQuery}`),
          api("/api/v1/compression/providers"),
        ]);
        state.compression.archives = archives;
        state.compression.providers = providers;
      }
      toast(t("refreshed"), t("compressionArchives"));
      render();
      break;
    case "set-home-hero-mode":
      setHomeHeroMode(data.mode || "auto", true);
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
      autoCollapseHomeHero();
      openSortOptionsModal();
      break;
    case "open-agent-filter":
      autoCollapseHomeHero();
      openAgentFilterModal();
      break;
    case "open-workspace-history":
      openWorkspaceHistoryModal();
      break;
    case "open-switch":
      openSwitchModal(data.provider, data.sessionId, data.workspace || state.home.workspace, data.title || "");
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
    case "copy-session-id":
      await copySessionId(data.copyText || "");
      break;
    case "open-manager-workspace":
      openManagerWorkspace(data.provider || "", data.workspace || "");
      break;
    case "toggle-detail-message":
      toggleDetailMessage(trigger);
      break;
    case "toggle-message-raw":
      toggleMessageRaw(trigger);
      break;
    case "scroll-to-message":
      scrollToDetailMessage(Number(data.messageIndex));
      break;
    case "open-sync-create":
      openSyncCreateModal(data.provider, data.sessionId, data.title || "");
      break;
    case "open-sync-rename":
      openSyncRenameModal(data.groupId, data.title || "");
      break;
    case "open-sync-remove":
      openSyncRemoveModal(data.groupId);
      break;
    case "open-sync-bind":
      openSyncBindModal(data.groupId);
      break;
    case "run-sync-latest":
      await runSyncGroup(data.groupId);
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
    case "open-manager-session-more":
      openManagerSessionMoreModal(JSON.parse(decodeURIComponent(data.value || "")));
      break;
    case "open-manager-workspace-more":
      openManagerWorkspaceMoreModal(managerWorkspaceItemFromValue(data.value || ""));
      break;
    case "open-manager-convert-workspace":
      openActionResultModal({
        title: t("convertWorkspaceWhole"),
        summary: t("workspaceConvertNotAvailable"),
      });
      break;
    case "run-codex-workspace-restore":
      await runCodexWorkspaceRestore(managerWorkspaceItemFromValue(data.value || ""));
      break;
    case "open-manager-clean-workspace-confirm":
      openManagerCleanWorkspaceConfirmModal();
      break;
    case "open-manager-backup-workspace-confirm":
      openManagerBackupWorkspaceConfirmModal();
      break;
    case "open-manager-clean-workspace-row-confirm":
      openManagerCleanWorkspaceConfirmModal(managerWorkspaceItemFromValue(data.value || ""));
      break;
    case "open-manager-backup-workspace-row-confirm":
      openManagerBackupWorkspaceConfirmModal(managerWorkspaceItemFromValue(data.value || ""));
      break;
    case "toggle-manager-selection-mode":
      state.manager.selectionMode = !state.manager.selectionMode;
      if (!state.manager.selectionMode) {
        state.manager.selectedItems = new Set();
      }
      render();
      break;
    case "toggle-manager-item":
      {
        const row = trigger.closest('.manager-row[data-action="toggle-manager-item"]');
        if (!state.manager.selectionMode) {
          const href = row?.dataset.href || "";
          if (href) navigate(href);
          break;
        }
        const value = data.value || "";
        const set = state.manager.selectedItems;
        if (set.has(value)) {
          set.delete(value);
          row?.classList.remove("is-selected");
        } else {
          set.add(value);
          row?.classList.add("is-selected");
        }
        updateManagerSelectionStats();
      }
      break;
    case "toggle-manager-workspace-item":
      {
        const value = data.value || "";
        const set = state.manager.selectedWorkspaceItems;
        const row = trigger.closest('.manager-row[data-action="toggle-manager-workspace-item"]');
        if (set.has(value)) {
          set.delete(value);
          row?.classList.remove("is-selected");
        } else {
          set.add(value);
          row?.classList.add("is-selected");
        }
        updateManagerSelectionStats();
      }
      break;
    case "select-all-manager":
      {
        state.manager.selectionMode = true;
        const rows = [...document.querySelectorAll('.manager-row[data-action="toggle-manager-item"]')];
        const allSelected = rows.length > 0 && rows.every((el) => state.manager.selectedItems.has(el.dataset.value));
        rows.forEach((el) => {
          if (allSelected) {
            state.manager.selectedItems.delete(el.dataset.value);
            el.classList.remove("is-selected");
          } else {
            state.manager.selectedItems.add(el.dataset.value);
            el.classList.add("is-selected");
          }
        });
        render();
      }
      break;
    case "select-all-manager-workspace":
      {
        const rows = [...document.querySelectorAll('.manager-row[data-action="toggle-manager-workspace-item"]')];
        const allSelected = rows.length > 0 && rows.every((el) => state.manager.selectedWorkspaceItems.has(el.dataset.value));
        rows.forEach((el) => {
          if (allSelected) {
            state.manager.selectedWorkspaceItems.delete(el.dataset.value);
            el.classList.remove("is-selected");
          } else {
            state.manager.selectedWorkspaceItems.add(el.dataset.value);
            el.classList.add("is-selected");
          }
        });
        updateManagerSelectionStats();
      }
      break;
    case "set-manager-view":
      {
        const view = data.view === "workspaces" ? "workspaces" : "sessions";
        state.manager.viewMode = view;
        const params = new URLSearchParams(window.location.search);
        if (view === "workspaces") {
          params.set("view", "workspaces");
        } else {
          params.delete("view");
        }
        replacePath(`/manager?${params.toString()}`);
        const draft = state.manager.draft || defaultManagerDraft();
        if (view === "workspaces" && !state.manager.quickWorkspacePreview && state.manager.isDefaultPreview) {
          // Lazy-load the quick workspace view on first switch
          if (draft.providers.length) {
            render(); // show loading state immediately
            const providerQuery = draft.providers.join(",");
            api(`/api/v1/manager/quick-workspaces?providers=${encodeURIComponent(providerQuery)}`)
              .then((quickWorkspacePreview) => {
                state.manager.quickWorkspacePreview = quickWorkspacePreview;
                render();
              })
              .catch((error) => toast(t("error"), error.message, true));
            return;
          }
        }
        if (!state.manager.isDefaultPreview) {
          if (view === "workspaces" && !state.manager.workspacePreview) {
            render();
            loadManagerWorkspacePreview(draft)
              .then((workspacePreview) => {
                state.manager.workspacePreview = workspacePreview;
                render();
              })
              .catch((error) => toast(t("error"), error.message, true));
            return;
          }
          if (view === "sessions" && !state.manager.preview) {
            render();
            loadManagerSessionPreview(draft)
              .then((preview) => {
                applyManagerDraftFilters(preview, draft);
                preview.output_dir = "";
                state.manager.preview = preview;
                render();
              })
              .catch((error) => toast(t("error"), error.message, true));
            return;
          }
        }
        render();
      }
      break;
    case "go-back":
      goBack();
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
      closeToast(Number(data.toastId));
      break;
    case "browse-folder":
      await browseFolderForField(trigger);
      break;
    case "browse-file":
      await browseFileForField(trigger);
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
          state.home.hookFilter = String(formData.get("hook_filter") || "all");
          state.home.visible = Number(formData.get("visible") || state.meta.settings.sessions_per_provider);
          autoCollapseHomeHero();
          if (workspaceChanged) {
            await setWorkspace(workspace);
          } else {
            await refreshHomeSessions();
          }
        }
        break;
      case "home-search":
        state.home.search = String(formData.get("search") || "");
        autoCollapseHomeHero();
        render();
        break;
      case "workspace-switch":
        await setWorkspace(String(formData.get("workspace") || "").trim());
        closeModal();
        break;
      case "home-list-options":
        state.home.sort = String(formData.get("sort") || "recent");
        state.home.hookFilter = String(formData.get("hook_filter") || "all");
        state.home.visible = Number(formData.get("visible") || state.meta.settings.sessions_per_provider);
        autoCollapseHomeHero();
        closeModal();
        await refreshHomeSessions();
        break;
      case "run-hook-operation":
        {
          const provider = String(formData.get("provider") || "");
          const settingId = String(formData.get("setting_id") || "");
          closeModal();
          await runHookOperation(provider, settingId);
        }
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
      case "create-sync":
        await runSyncCreate(formData);
        break;
      case "rename-sync":
        await runSyncRename(formData);
        break;
      case "remove-sync":
        await runSyncRemove(formData);
        break;
      case "bind-sync":
        await runSyncBind(formData);
        break;
      case "sync-from-sync":
        await runSyncGroup(String(formData.get("group_id")), String(formData.get("holding_id")));
        break;
      case "unbind-sync":
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
      case "clean-manager-workspace":
        await runManagerWorkspaceClean();
        break;
      case "backup-manager-workspace":
        await runManagerWorkspaceBackup();
        break;
      case "restore-compression":
        await runCompressionRestore(formData);
        break;
      case "expand-compression":
        await runCompressionExpand(formData);
        break;
      case "compress-session":
        await runCompressSession(formData);
        break;
      default:
        break;
    }
  } catch (error) {
    toast(t("error"), error.message, true);
  }
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

function managerWorkspaceItemFromValue(value) {
  if (!value) return null;
  try {
    return JSON.parse(decodeURIComponent(value));
  } catch (_error) {
    return null;
  }
}

function openManagerCleanWorkspaceConfirmModal(item = null) {
  const items = item ? [item] : selectedManagerWorkspaceItems();
  if (!items.length) {
    toast(t("error"), t("noSelection"), true);
    return;
  }
  state.manager.pendingItems = items;
  const totalSessions = items.reduce((sum, item) => sum + Number(item.session_count || 0), 0);
  state.modal = {
    kind: "form",
    title: t("cleanWorkspaceSelected"),
    submit: "clean-manager-workspace",
    submitClass: "danger",
    submitLabel: t("confirm"),
    body: `
      <div class="stack">
        <p>${t("cleanWorkspaceConfirm")}</p>
        <div class="path-line">${t("workspaceStat")}: ${items.length}</div>
        <div class="path-line">${t("sessionsStat")}: ${totalSessions}</div>
      </div>`,
  };
  render();
}

function openManagerBackupWorkspaceConfirmModal(item = null) {
  const items = item ? [item] : selectedManagerWorkspaceItems();
  if (!items.length) {
    toast(t("error"), t("noSelection"), true);
    return;
  }
  state.manager.pendingItems = items;
  const outputDir = state.meta.settings.default_backup_dir || "./backups";
  const totalSessions = items.reduce((sum, item) => sum + Number(item.session_count || 0), 0);
  state.modal = {
    kind: "form",
    title: t("backupWorkspaceSelected"),
    submit: "backup-manager-workspace",
    submitLabel: t("confirm"),
    body: `
      <div class="stack">
        <p>${t("backupWorkspaceConfirm")}</p>
        <div class="path-line">${t("workspaceStat")}: ${items.length}</div>
        <div class="path-line">${t("sessionsStat")}: ${totalSessions}</div>
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
  const importedProviderId = String(formData.get("provider"));
  openActionResultModal({
    title: t("imported"),
    summary: providers.displayName(importedProviderId),
    lines: [
      `${t("target")}: ${providers.displayName(importedProviderId)}`,
      `${t("sessionId")}: ${result.new_session_id}`,
      ...(result.resume_command ? [`${t("resumeCommand")}: ${result.resume_command}`] : []),
    ],
    navPath: `/sessions/${encodeURIComponent(importedProviderId)}/${encodeURIComponent(result.new_session_id)}`,
  });
}

async function runSwitch(formData) {
  const fromProvider = String(formData.get("from"));
  const toProvider = String(formData.get("to"));
  const explicitAction = String(formData.get("action") || "").toLowerCase();
  const moveOriginal = explicitAction === "move";
  const targetTitle = String(formData.get("target_title") || "").trim();
  setLoading(true, {
    label: moveOriginal ? t("moveAction") : t("copyAction"),
    detail: t("targetProvider"),
  });
  try {
    const result = await api("/api/v1/switch", {
      method: "POST",
      body: {
        from: fromProvider,
        to: toProvider,
        session_id: emptyToNull(formData.get("session_id")),
        to_dir: emptyToNull(formData.get("to_dir")),
        target_title: emptyToNull(targetTitle),
        move_original: moveOriginal,
      },
    });
    await loadHome();
    closeModal();
    openActionResultModal({
      title: moveOriginal ? t("moveAction") : t("copyAction"),
      summary: `${result.from_name} → ${result.to_name}`,
      lines: [
        `${t("source")}: ${result.from_name} / ${result.source_session_id}`,
        `${t("target")}: ${result.to_name} / ${result.target_session_id}`,
        ...(targetTitle ? [`${t("title")}: ${targetTitle}`] : []),
        ...(moveOriginal ? [t("removeOriginalSession")] : []),
        ...(result.resume_command ? [`${t("resumeCommand")}: ${result.resume_command}`] : []),
      ],
      navPath: `/sessions/${encodeURIComponent(toProvider)}/${encodeURIComponent(result.target_session_id)}`,
    });
  } finally {
    setLoading(false);
  }
}

async function runExport(formData) {
  const result = await api("/api/v1/export", {
    method: "POST",
    body: {
      provider: String(formData.get("provider")),
      session_id: String(formData.get("session_id")),
      output_prefix: emptyToNull(formData.get("output_prefix")),
      format: String(formData.get("format")),
      output_dir: emptyToNull(formData.get("output_dir")),
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

async function runCompressSession(formData) {
  const provider = String(formData.get("provider") || "");
  const sessionId = String(formData.get("session_id") || "");
  if (!provider || !sessionId) throw new Error(t("missingSessionInfo"));
  closeModal();
  setLoading(true, { label: t("compressSession"), detail: `${provider} / ${sessionId}` });
  try {
    const result = await api("/api/v1/compression/apply", {
      method: "POST",
      body: {
        source_provider_id: provider,
        target_provider_id: provider,
        session_id: sessionId,
        policy: {
          protect_recent_message_events: 6,
          min_candidate_bytes: 4096,
          min_savings_ratio_percent: 20,
          mode: "auto",
        },
      },
    });
    await loadRoute();
    const report = result.report || {};
    const archiveRefs = result.archive_refs || [];
    const candidates = report.candidates || [];
    const saved = report.estimated_bytes_saved || 0;
    openActionResultModal({
      title: t("compressSessionComplete") || t("compressionTitle"),
      summary: `${candidates.length} ${t("segments")}, ${formatBytes(saved)} ${t("saved")}`,
      lines: archiveRefs.length ? archiveRefs : result.files || [],
      navPath: `/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`,
      navLabel: t("openDetail"),
    });
  } finally {
    setLoading(false);
  }
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

async function runSyncCreate(formData) {
  const provider = String(formData.get("provider"));
  const targets = [...new Set(formData.getAll("targets").map(String).filter((target) => target && target !== provider))];
  if (!targets.length) {
    toast(t("error"), t("noSyncTargets"), true);
    return;
  }
  const result = await api("/api/v1/sync", {
    method: "POST",
    body: {
      provider,
      session_id: String(formData.get("session_id")),
      targets,
      to_dir: emptyToNull(formData.get("to_dir")),
      title: emptyToNull(formData.get("title")),
    },
  });
  await loadHome();
  closeModal();
  openActionResultModal({
    title: t("syncCreated"),
    summary: result.id,
    lines: [
      `${t("sessionId")}: ${result.id}`,
      `${t("holdings")}: ${result.holdings.length}`,
    ],
    navPath: `/sync/${encodeURIComponent(result.id)}`,
    navLabel: t("openDetail"),
  });
}

async function runSyncRename(formData) {
  const groupId = String(formData.get("group_id"));
  const title = String(formData.get("title"));
  await api(`/api/v1/sync/${encodeURIComponent(groupId)}`, {
    method: "PATCH",
    body: { title },
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("rename"),
    summary: t("syncTitle"),
    lines: [
      `${t("sessionId")}: ${groupId}`,
      `${t("title")}: ${title}`,
    ],
    navPath: `/sync/${encodeURIComponent(groupId)}`,
    navLabel: t("openDetail"),
  });
}

async function runSyncRemove(formData) {
  const groupId = String(formData.get("group_id"));
  const removeUrl = new URL(`/api/v1/sync/${encodeURIComponent(groupId)}`, window.location.origin);
  removeUrl.searchParams.set("delete_provider_sessions", formData.get("delete_provider_sessions") ? "true" : "false");
  await api(removeUrl.pathname + removeUrl.search, { method: "DELETE" });
  state.home.syncGroups = (state.home.syncGroups || []).filter((group) => group.id !== groupId);
  if (state.route.name === "sync-detail" && state.route.groupId === groupId) {
    replacePath("/sync");
    state.syncDetail = null;
  }
  closeModal();
  openActionResultModal({
    title: t("deleted"),
    lines: [
      `${t("sessionId")}: ${groupId}`,
      `${t("syncTitle")}: ${groupId}`,
    ],
    navPath: "/sync",
    navLabel: t("syncGroups"),
  });
}

async function runSyncBind(formData) {
  const provider = String(formData.get("provider"));
  const sessionId = emptyToNull(formData.get("session_id"));
  if (!sessionId && !providers.capabilitySet(provider).export) {
    toast(t("error"), t("noSyncTargets"), true);
    return;
  }

  const result = await api("/api/v1/sync/bind", {
    method: "POST",
    body: {
      group_id: String(formData.get("group_id")),
      provider,
      session_id: sessionId,
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

async function runSyncGroup(groupId, sourceHoldingId = null) {
  const result = await api("/api/v1/sync/sync", {
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
  await api(`/api/v1/sync/holdings/${encodeURIComponent(groupId)}/${encodeURIComponent(holdingId)}`, {
    method: "DELETE",
  });
  await loadRoute();
  closeModal();
  openActionResultModal({
    title: t("unbind"),
    summary: t("deleted"),
    lines: [
      `${t("sessionId")}: ${holdingId}`,
      `${t("syncTitle")}: ${groupId}`,
    ],
    navPath: `/sync/${encodeURIComponent(groupId)}`,
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
      compress: formData.get("home_button_compress") === "true",
      switch: formData.get("home_button_switch") === "true",
      export: formData.get("home_button_export") === "true",
      sync: formData.get("home_button_sync") === "true",
      delete: formData.get("home_button_delete") === "true",
    },
    agent_order: formData.getAll("agent_order").map(String),
    primary_agents: [],
    hidden_agents: formData.getAll("hidden_agents").map(String),
  };
  await saveSettings(body);
  closeModal();
}

async function saveSettings(body) {
  await api("/api/v1/settings", {
    method: "PUT",
    body,
  });
  await api("/api/v1/providers/catalog", {
    method: "PUT",
    body: {
      sort_order: { global: body.agent_order, workspace: [] },
      hidden_state: { global: body.hidden_agents, workspace: [] },
    },
  });
  state.meta = await api("/api/v1/meta");
  invalidateCatalogClientCache();
  await refreshCatalog(state.home.workspace, { force: true });
  setDocumentLanguage();
  state.home.visible = state.meta.settings.sessions_per_provider;
  toast(t("saved"), t("settingsTitle"));
  render();
}

async function loadManagerData(draft) {
  const body = managerPreviewBody(draft);
  const [preview, workspacePreview] = await Promise.all([
    api("/api/v1/manager/preview", { method: "POST", body }),
    api("/api/v1/manager/workspaces", { method: "POST", body }),
  ]);
  return { preview, workspacePreview };
}

async function loadManagerSessionPreview(draft) {
  return api("/api/v1/manager/preview", { method: "POST", body: managerPreviewBody(draft) });
}

async function loadManagerWorkspacePreview(draft) {
  return api("/api/v1/manager/workspaces", { method: "POST", body: managerPreviewBody(draft) });
}

async function loadManagerRoutePreview(route) {
  if (route.workspace && route.workspace !== state.home.workspace) {
    await setWorkspace(route.workspace, { reloadManager: false, loadHome: false });
  }

  const baseDraft = defaultManagerDraft();
  const providers = route.provider ? [route.provider] : baseDraft.providers;
  const draft = {
    ...baseDraft,
    workspace: route.workspace || state.home.workspace || "",
    providers,
  };
  const viewMode = route.view === "workspaces" ? "workspaces" : "sessions";
  const [preview, workspacePreview] = await Promise.all([
    viewMode === "sessions" ? loadManagerSessionPreview(draft) : Promise.resolve(null),
    viewMode === "workspaces" ? loadManagerWorkspacePreview(draft) : Promise.resolve(null),
  ]);
  state.manager = {
    ...state.manager,
    draft,
    preview,
    workspacePreview,
    quickPreview: null,
    quickWorkspacePreview: null,
    isDefaultPreview: false,
    stats: null,
    statsLoading: true,
    statsRequestId: "",
    report: null,
    pendingItems: [],
    viewMode,
    selectedItems: new Set(),
    selectedWorkspaceItems: new Set(),
  };
  render();
  queueManagerStatsLoad(draft);
}

function openManagerWorkspace(provider, workspace) {
  const params = new URLSearchParams({ view: "sessions" });
  if (provider) params.set("provider", provider);
  if (workspace) params.set("workspace", workspace);
  navigate(`/manager?${params.toString()}`);
}

async function refreshManagerData(draft) {
  if (state.manager.isDefaultPreview) {
    const providerQuery = draft.providers.join(",");
    const [quickPreview, quickWorkspacePreview] = await Promise.all([
      api(`/api/v1/manager/quick-preview?providers=${encodeURIComponent(providerQuery)}`),
      state.manager.viewMode === "workspaces"
        ? api(`/api/v1/manager/quick-workspaces?providers=${encodeURIComponent(providerQuery)}`)
        : Promise.resolve(null),
    ]);
    return { quickPreview, quickWorkspacePreview, isDefaultPreview: true };
  }
  if (state.manager.viewMode === "workspaces") {
    const workspacePreview = await loadManagerWorkspacePreview(draft);
    return { preview: state.manager.preview, workspacePreview, isDefaultPreview: false };
  }
  const preview = await loadManagerSessionPreview(draft);
  return { preview, workspacePreview: state.manager.workspacePreview, isDefaultPreview: false };
}

function queueManagerStatsLoad(draft) {
  const body = managerPreviewBody(draft);
  const requestKey = JSON.stringify(body);
  if (state.manager.statsLoading && state.manager.statsRequestId && state.manager.statsRequestKey === requestKey) return;
  if (state.manager.stats && state.manager.statsRequestKey === requestKey) return;

  const requestId = `${Date.now()}:${Math.random()}`;
  state.manager.stats = null;
  state.manager.statsLoading = true;
  state.manager.statsRequestId = requestId;
  state.manager.statsRequestKey = requestKey;

  void api("/api/v1/manager/stats", { method: "POST", body })
    .then((stats) => {
      if (state.manager.statsRequestId !== requestId) return;
      state.manager.stats = stats;
      state.manager.statsLoading = false;
      if (state.route.name === "manager") render();
    })
    .catch(() => {
      if (state.manager.statsRequestId !== requestId) return;
      state.manager.statsLoading = false;
      if (state.route.name === "manager") render();
    });
}

async function loadDefaultManagerPreview() {
  const draft = state.manager.draft || defaultManagerDraft();
  if (!draft.providers.length) {
    state.manager = {
      ...state.manager,
      draft,
      quickPreview: emptyManagerPreview(),
      quickWorkspacePreview: null,
      preview: null,
      workspacePreview: null,
      isDefaultPreview: true,
      stats: null,
      statsLoading: false,
      statsRequestId: "",
      report: null,
      pendingItems: [],
      selectionMode: false,
      selectedItems: new Set(),
      selectedWorkspaceItems: new Set(),
    };
    render();
    return;
  }
  const quickPreview = await api("/api/v1/manager/quick-preview");
  state.manager = {
    ...state.manager,
    draft,
    quickPreview,
    quickWorkspacePreview: null,
    preview: null,
    workspacePreview: null,
    isDefaultPreview: true,
    stats: null,
    statsLoading: true,
    statsRequestId: "",
    report: null,
    pendingItems: [],
    selectedItems: new Set(),
    selectedWorkspaceItems: new Set(),
  };
  render();
  queueManagerStatsLoad(draft);
}

async function runManagerPreview(formData) {
  const draft = managerDraftFromFormData(formData);
  if (!draft.providers.length) throw new Error(t("noTargetAgentSelected"));
  setLoading(true, { label: t("managerPreview"), detail: t("scanningSessions") });
  try {
    const viewMode = state.manager.viewMode || "sessions";
    const [preview, workspacePreview] = await Promise.all([
      viewMode === "sessions" ? loadManagerSessionPreview(draft) : Promise.resolve(state.manager.preview),
      viewMode === "workspaces" ? loadManagerWorkspacePreview(draft) : Promise.resolve(state.manager.workspacePreview),
    ]);
    if (preview) {
      applyManagerDraftFilters(preview, draft);
      preview.output_dir = "";
    }
    state.manager = {
      ...state.manager,
      draft,
      preview: viewMode === "sessions" ? preview : null,
      workspacePreview: viewMode === "workspaces" ? workspacePreview : null,
      isDefaultPreview: false,
      stats: null,
      statsLoading: true,
      statsRequestId: "",
      report: null,
      pendingItems: [],
      viewMode,
      selectedItems: new Set(),
      selectedWorkspaceItems: new Set(),
    };
    state.modal = null;
    if (state.route.name !== "manager") replacePath("/manager");
    render();
    queueManagerStatsLoad(draft);
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
    const refreshResult = await refreshManagerData(draft);
    applyRefreshResult(refreshResult, draft);
    openActionResultModal({
      title: t("cleanSelected"),
      summary: `${result.success} ${t("managerCleaned")}, ${result.failed} ${t("managerFailed")}, ${formatBytes(
        result.freed_bytes
      )} ${t("managerFreed")}`,
      lines: result.errors || [],
    });
    queueManagerStatsLoad(draft);
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
    const refreshResult = await refreshManagerData(draft);
    applyRefreshResult(refreshResult, draft, { outputDir });
    openActionResultModal({
      title: t("backupSelected"),
      summary: `${result.success} ${t("managerExported")}, ${result.failed} ${t("managerFailed")}`,
      lines: [...(result.files || []), ...(result.errors || [])],
    });
    queueManagerStatsLoad(draft);
  } finally {
    setLoading(false);
  }
}

function selectedManagerItems() {
  const visible = new Set(
    [...document.querySelectorAll('.manager-row[data-action="toggle-manager-item"]')].map((el) => el.dataset.value)
  );
  return [...(state.manager?.selectedItems || new Set())]
    .filter((value) => visible.has(value))
    .map((value) => JSON.parse(decodeURIComponent(value)));
}

// Apply the result of refreshManagerData() back into state.manager.
// refreshResult is either { isDefaultPreview: true, quickPreview, quickWorkspacePreview }
// or { isDefaultPreview: false, preview, workspacePreview }.
function applyRefreshResult(refreshResult, draft, { outputDir = "", report = null } = {}) {
  if (refreshResult.isDefaultPreview) {
    state.manager = {
      ...state.manager,
      draft,
      quickPreview: refreshResult.quickPreview,
      quickWorkspacePreview: refreshResult.quickWorkspacePreview,
      preview: null,
      workspacePreview: null,
      isDefaultPreview: true,
      stats: null,
      statsLoading: true,
      statsRequestId: "",
      report,
      pendingItems: [],
      selectionMode: false,
      selectedItems: new Set(),
      selectedWorkspaceItems: new Set(),
    };
  } else {
    const preview = refreshResult.preview;
    const viewMode = state.manager.viewMode || "sessions";
    if (preview) {
      applyManagerDraftFilters(preview, draft);
      preview.output_dir = outputDir;
    }
    state.manager = {
      ...state.manager,
      draft,
      preview: viewMode === "sessions" ? preview : null,
      workspacePreview: viewMode === "workspaces" ? refreshResult.workspacePreview : null,
      isDefaultPreview: false,
      stats: null,
      statsLoading: true,
      statsRequestId: "",
      report,
      pendingItems: [],
      selectionMode: false,
      selectedItems: new Set(),
      selectedWorkspaceItems: new Set(),
    };
  }
}

async function runManagerWorkspaceClean() {
  const draft = readManagerDraft();
  const items = state.manager.pendingItems.length ? state.manager.pendingItems : selectedManagerWorkspaceItems();
  if (!items.length) throw new Error(t("noSelection"));
  closeModal();
  setLoading(true, { label: t("cleanWorkspaceSelected"), detail: `${items.length} ${t("workspaceStat")}`, progress: 0.12 });
  try {
    const results = [];
    let totalSuccess = 0;
    let totalFailed = 0;
    let totalFreed = 0;
    let errors = [];
    for (const item of items) {
      const result = await api("/api/v1/manager/clean-workspace", {
        method: "POST",
        body: { provider_id: item.provider_id, workspace: item.workspace },
      });
      totalSuccess += result.success;
      totalFailed += result.failed;
      totalFreed += result.freed_bytes;
      errors = errors.concat(result.errors || []);
      results.push(`${item.provider_id} / ${workspaceName(item.workspace) || item.workspace}: ${result.success}/${result.failed}`);
    }
    updateLoading({ detail: t("refreshingSessions"), progress: 0.82 });
    const refreshResult = await refreshManagerData(draft);
    applyRefreshResult(refreshResult, draft);
    openActionResultModal({
      title: t("cleanWorkspaceSelected"),
      summary: `${totalSuccess} ${t("managerCleaned")}, ${totalFailed} ${t("managerFailed")}, ${formatBytes(totalFreed)} ${t("managerFreed")}`,
      lines: [...results, ...errors],
    });
    queueManagerStatsLoad(draft);
  } finally {
    setLoading(false);
  }
}

async function runManagerWorkspaceBackup() {
  const draft = readManagerDraft();
  const items = state.manager.pendingItems.length ? state.manager.pendingItems : selectedManagerWorkspaceItems();
  if (!items.length) throw new Error(t("noSelection"));
  const outputDir = state.meta.settings.default_backup_dir || "./backups";
  closeModal();
  setLoading(true, { label: t("backupWorkspaceSelected"), detail: `${items.length} ${t("workspaceStat")}`, progress: 0.12 });
  try {
    const results = [];
    let totalSuccess = 0;
    let totalFailed = 0;
    let files = [];
    let errors = [];
    for (const item of items) {
      const result = await api("/api/v1/manager/backup-workspace", {
        method: "POST",
        body: { provider_id: item.provider_id, workspace: item.workspace, output_dir: outputDir },
      });
      totalSuccess += result.success;
      totalFailed += result.failed;
      files = files.concat(result.files || []);
      errors = errors.concat(result.errors || []);
      results.push(`${item.provider_id} / ${workspaceName(item.workspace) || item.workspace}: ${result.success}/${result.failed}`);
    }
    updateLoading({ detail: t("refreshingSessions"), progress: 0.82 });
    const refreshResult = await refreshManagerData(draft);
    applyRefreshResult(refreshResult, draft, { outputDir });
    openActionResultModal({
      title: t("backupWorkspaceSelected"),
      summary: `${totalSuccess} ${t("managerExported")}, ${totalFailed} ${t("managerFailed")}`,
      lines: [...files, ...results, ...errors],
    });
    queueManagerStatsLoad(draft);
  } finally {
    setLoading(false);
  }
}

function defaultManagerDraft() {
  const selectedProviders = getOrderedProviders()
    .filter((item) => !providers.isHiddenGlobal(item))
    .filter((item) => providers.hasFilter(item, "is_installed"))
    .map((item) => item.provider_id);
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
      compress: formData.get("home_button_compress") === "true",
      switch: formData.get("home_button_switch") === "true",
      export: formData.get("home_button_export") === "true",
      sync: formData.get("home_button_sync") === "true",
      delete: formData.get("home_button_delete") === "true",
    },
    agent_order: formData.getAll("agent_order").map(String),
    primary_agents: [],
    hidden_agents: formData.getAll("hidden_agents").map(String),
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

function render() {
  appEl.innerHTML = renderAppShell({
    state,
    t,
    escapeHtml,
    renderPage,
    renderLoading,
    renderToasts,
    renderTopbarContext,
  });
  modalRoot.innerHTML = renderModalMarkup(state, t, escapeHtml, escapeAttr);
  bindLocalButtons();
  navigation.restorePageState();
}

function renderTopbarContext() {
  return renderTopbarContextMarkup(state, providers, t, escapeHtml);
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
  const wrapPage = (markup) => `<div class="${routeScrollClass(state.route)}">${markup}</div>`;
  switch (state.route.name) {
    case "home":
      return renderHome();
    case "session":
      return wrapPage(renderSessionDetail());
    case "sync-list":
      return wrapPage(renderSyncList());
    case "sync-detail":
      return wrapPage(renderSyncDetail());
    case "manager":
      return wrapPage(renderManagerPage());
    case "compression":
      return wrapPage(renderCompressionPage());
    case "hooks":
      return wrapPage(renderHooksCenterPage());
    case "agents":
      return wrapPage(renderAgentManagementPage());
    default:
      return `<div class="page-scroll"><div class="empty-state">${t("notFound")}</div></div>`;
  }
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
    await loadAgentProviderDetail(providerId, { refresh: true });
    toast(t("saved"), agentSettingLabel({ id: settingId, title: settingId }));
    render();
  } catch (error) {
    toast(t("error"), error.message, true);
  }
}

function openHookOperationConfirm(providerId, settingId) {
  const provider = (state.hooks.overview?.providers || state.agents.providers || []).find(
    (item) => item.provider_id === providerId
  );
  const hook = provider?.hook || {};
  const profile = provider?.hook_profile || {};
  const providerName = providers.displayName(providerId);
  const configPath = hook.config_path || profile.config_hint || "—";
  const operationLabel = hookOperationLabel(settingId);
  const isUninstall = settingId === "uninstall_hook";
  state.modal = {
    kind: "form",
    title: t("hookOperationConfirmTitle"),
    submit: "run-hook-operation",
    submitLabel: operationLabel,
    submitClass: isUninstall ? "danger" : "invert",
    body: `
      <input type="hidden" name="provider" value="${escapeAttr(providerId)}">
      <input type="hidden" name="setting_id" value="${escapeAttr(settingId)}">
      <div class="stack">
        <p class="muted">${escapeHtml(t("hookOperationConfirmHint"))}</p>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("provider"), providerName)}
          ${renderMetaLine(t("operation"), operationLabel)}
          ${renderMetaLine("Hook status", hook.status || "unknown")}
          ${renderMetaLine("Hook format", profile.format || "—")}
          ${renderMetaLine("Current version", hook.current_version || "—")}
          ${renderMetaLine("Installed version", hook.installed_version || "—")}
          ${renderMetaLine("Hook config", configPath)}
        </div>
        <div class="empty-state">${escapeHtml(operationImpactText(settingId))}</div>
      </div>`,
  };
  render();
}

function hookOperationLabel(settingId) {
  switch (settingId) {
    case "install_hook":
      return t("hookInstall");
    case "repair_hook":
      return t("hookRepair");
    case "uninstall_hook":
      return t("hookUninstall");
    case "verify_hook":
      return t("hookVerify");
    default:
      return settingId;
  }
}

function operationImpactText(settingId) {
  switch (settingId) {
    case "install_hook":
      return t("hookInstallImpact");
    case "repair_hook":
      return t("hookRepairImpact");
    case "uninstall_hook":
      return t("hookUninstallImpact");
    default:
      return t("hookOperationImpact");
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
    if (state.route.name === "hooks") {
      await loadHooksCenter();
    } else {
      await Promise.all([
        loadAgentProviderDetail(providerId, { refresh: true }),
        api("/api/v1/hooks/runtime-sessions").then((sessions) => {
          state.agents.hookRuntimeSessions = sessions || [];
        }).catch(() => {}),
      ]);
    }
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

async function runCodexWorkspaceRestore(item) {
  if (!item?.workspace) return;
  if (item.provider_id !== "codex") {
    toast(t("error"), t("codexRestoreWorkspaceOnly"), true);
    return;
  }
  const settingId = "repair_workspace_sessions";
  const key = `codex:${settingId}:${item.workspace}`;
  if (state.agents.pendingSettings[key]) return;
  state.agents.pendingSettings[key] = true;
  closeModal();
  setLoading(true, {
    label: t("codexRestoreWorkspaceSessions"),
    detail: workspaceName(item.workspace) || item.workspace,
  });
  try {
    const result = await api(`/api/v1/providers/codex/settings/${encodeURIComponent(settingId)}`, {
      method: "POST",
      body: {
        workspace: item.workspace,
      },
    });
    state.agents.settingResults[key] = result;
    updateLoading({ detail: t("refreshingSessions") });
    const draft = readManagerDraft();
    const refreshResult = await refreshManagerData(draft);
    applyRefreshResult(refreshResult, draft);
    const report = result?.data || result;
    openActionResultModal({
      title: t("codexRestoreWorkspaceSessions"),
      summary: t("codexRestoreWorkspaceComplete"),
      lines: codexWorkspaceRestoreLines(report),
    });
    queueManagerStatsLoad(draft);
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    delete state.agents.pendingSettings[key];
    setLoading(false);
    render();
  }
}

function codexWorkspaceRestoreLines(report) {
  const data = report?.type === "codex_workspace_repair" ? report.data : report;
  if (!data || typeof data !== "object") return [];
  const lines = [
    `Workspace: ${data.workspace_dir || "—"}`,
    `Current provider: ${data.current_model_provider || "—"}`,
    `Scanned rollout files: ${data.scanned_rollouts ?? 0}`,
    `Workspace sessions: ${data.workspace_session_count ?? 0}`,
    `Hidden sessions: ${data.hidden_session_count ?? 0}`,
    `Repaired sessions: ${data.repaired_session_count ?? 0}`,
    `Reindexed sessions: ${data.reindexed_session_count ?? 0}`,
    `Updated SQLite rows: ${data.sqlite_rows_updated ?? 0}`,
  ];
  if (data.backup_dir) lines.push(`Backup: ${data.backup_dir}`);
  if (Array.isArray(data.touched_sessions) && data.touched_sessions.length) {
    lines.push("");
    data.touched_sessions.slice(0, 20).forEach((session) => {
      lines.push(`${session.session_id || "—"} | ${session.title || "(untitled)"}`);
    });
  }
  return lines;
}

async function runHookOperation(providerId, settingId) {
  const key = `${providerId}:${settingId}`;
  if (state.agents.pendingSettings[key]) return;
  state.agents.pendingSettings[key] = true;
  setLoading(true, {
    label: hookOperationLabel(settingId),
    detail: providerId || "",
  });
  try {
    const operation = hookOperationApiName(settingId);
    const result = await api(
      `/api/v1/hooks/providers/${encodeURIComponent(providerId)}/operations/${encodeURIComponent(operation)}`,
      { method: "POST" }
    );
    state.agents.settingResults[key] = { type: "hook_operation", data: result };
    if (state.route.name === "hooks") {
      await loadHooksCenter();
    } else {
      await loadAgentProviderDetail(providerId, { refresh: true });
    }
    toast(t("done"), hookOperationLabel(settingId));
  } catch (error) {
    toast(t("error"), error.message, true);
  } finally {
    delete state.agents.pendingSettings[key];
    setLoading(false);
    render();
  }
}

function hookOperationApiName(settingId) {
  switch (settingId) {
    case "install_hook":
      return "install";
    case "verify_hook":
      return "verify";
    case "repair_hook":
      return "repair";
    case "uninstall_hook":
      return "uninstall";
    default:
      return settingId;
  }
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
      <div class="modal-form-grid">
        <label class="field span-full">
          <span>${t("olderThanDays")}</span>
          <div class="input-unit-combo">
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
        </label>
        <label class="field">
          <span>${t("storageGreaterThan")}</span>
          <div class="input-unit-combo">
            <input type="text" inputmode="decimal" name="size_min_value" placeholder="1" value="${escapeAttr(
              draft.size_min_value
            )}">
            <select name="size_min_unit">
              ${unitOption("kb", "KB", draft.size_min_unit)}
              ${unitOption("mb", "MB", draft.size_min_unit)}
              ${unitOption("gb", "GB", draft.size_min_unit)}
            </select>
          </div>
        </label>
        <label class="field">
          <span>${t("storageLessThan")}</span>
          <div class="input-unit-combo">
            <input type="text" inputmode="decimal" name="size_max_value" placeholder="10" value="${escapeAttr(
              draft.size_max_value
            )}">
            <select name="size_max_unit">
              ${unitOption("kb", "KB", draft.size_max_unit)}
              ${unitOption("mb", "MB", draft.size_max_unit)}
              ${unitOption("gb", "GB", draft.size_max_unit)}
            </select>
          </div>
        </label>
        <label class="field">
          <span>${t("managerMaxResults")}</span>
          <input type="text" inputmode="numeric" name="max_results" value="${escapeAttr(draft.max_results || "")}" placeholder="10">
        </label>
        <label class="field">
          <span>${t("managerSortOrder")}</span>
          <select name="sort_order">
            ${unitOption("recent", t("managerSortRecentDesc"), draft.sort_order || "recent")}
            ${unitOption("size", t("managerSortSizeDesc"), draft.sort_order || "recent")}
          </select>
        </label>
        <label class="field">
          <span>${t("titleContains")}</span>
          <input name="title_contains" value="${escapeAttr(draft.title_contains || "")}" placeholder="${escapeAttr(t("keywordPlaceholder"))}">
        </label>
        <label class="field">
          <span>${t("titleExcludes")}</span>
          <input name="title_excludes" value="${escapeAttr(draft.title_excludes || "")}" placeholder="${escapeAttr(t("keywordPlaceholder"))}">
        </label>
      </div>
      <p class="modal-form-hint muted">${t("managerFilterFutureHint")}</p>`,
    submitLabel: t("preview"),
  };
  render();
}

function renderLoading() {
  return renderLoadingMarkup(state, t, escapeHtml);
}

function renderToasts() {
  return renderToastsMarkup(state, t, escapeHtml, escapeAttr);
}

function closeToast(index) {
  closeChromeToast(state, index, render);
}

function closeModal() {
  closeChromeModal(state, render);
}

async function copyDetailMessage(index) {
  const event = state.session?.view?.events?.[index];
  if (!event) return;
  const text = detailEventToText(event);
  try {
    await navigator.clipboard.writeText(text);
    toast(t("copied"), t("copy"));
  } catch (error) {
    toast(t("error"), t("clipboardCopyFailed") + ": " + (error.message || String(error)), true);
  }
}

async function copySessionId(text) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    toast(t("copied"), text);
  } catch (error) {
    toast(t("error"), t("clipboardCopyFailed") + ": " + (error.message || String(error)), true);
  }
}

function toggleDetailMessage(trigger) {
  const item = trigger?.closest?.(".msg-item");
  if (!item) return;
  const isExpanded = item.classList.toggle("is-expanded");
  trigger.textContent = isExpanded ? t("collapse") : t("expand");
}

function toggleMessageRaw(trigger) {
  const item = trigger?.closest?.(".msg-item");
  if (!item) return;
  const isRaw = item.classList.toggle("is-raw");
  item.querySelectorAll(".content-block").forEach((block) => {
    block.classList.toggle("is-raw", isRaw);
  });
  trigger.textContent = isRaw ? t("viewFormatted") : t("viewRaw");
}

function scrollToDetailMessage(index) {
  const item = document.querySelector(`.msg-item[data-message-index="${index}"]`);
  if (!item) return;
  item.scrollIntoView({ behavior: "smooth", block: "start" });
  item.classList.add("is-highlighted");
  window.setTimeout(() => item.classList.remove("is-highlighted"), 1200);
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
  showToast(state, title, message, error, render);
}

function fatal(error) {
  renderFatal(appEl, error, t, escapeHtml);
}

function providerOptions(skipId = "", selectedId = "") {
  return homeProviderOptions(skipId, selectedId);
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

const {
  openActionResultModal,
  openAgentFilterModal,
  openCompressSessionModal,
  openCompressionExpandModal,
  openCompressionRestoreModal,
  openDeleteModal,
  openExportModal,
  openImportModal,
  openManagerSessionMoreModal,
  openManagerWorkspaceMoreModal,
  openPushSyncModal,
  openRenameModal,
  openSyncCreateModal,
  openSyncBindModal,
  openSyncRemoveModal,
  openSyncRenameModal,
  openSortOptionsModal,
  openSwitchModal,
  openSyncResultModal,
  openUnbindModal,
  openWorkspaceHistoryModal,
  openWorkspaceSwitchModal,
} = createModalModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  workspaceName,
  render: () => render(),
  getOrderedProviders,
  getDefaultSwitchTarget,
  homeProviderOptions,
  renderPathField,
  renderWorkspaceDatalist,
});

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

async function browseFileForField(trigger) {
  const fieldName = trigger?.dataset?.targetField;
  if (!fieldName) return;

  const scope = trigger.closest("form, .workspace-panel, .modal-card") || document;
  const selector = `[name="${fieldName}"]`;
  const input = scope.querySelector(selector) || document.querySelector(selector);
  if (!(input instanceof HTMLInputElement)) return;

  try {
    const result = await api("/api/v1/system/select-file", {
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
      ? t("filePickerUnavailable")
      : error.message;
    toast(t("error"), message, true);
  }
}

function setLoading(active, info = null) {
  setChromeLoading(state, active, info, render);
}

function updateLoading(info) {
  updateChromeLoading(state, info, render);
}
