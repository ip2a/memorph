import { ASCII_BANNER_COLORS } from "./constants.js";
import { parseRoute } from "./router.js";

export function randomAsciiBannerColor() {
  return ASCII_BANNER_COLORS[Math.floor(Math.random() * ASCII_BANNER_COLORS.length)];
}

export function createState() {
  return {
    catalog: { providers: [] },
    meta: null,
    route: parseRoute(window.location.pathname, new URLSearchParams(window.location.search)),
    loading: 0,
    loadingInfo: null,
    ui: {
      homeProviderVisibleCount: null,
      homeHeroMode: "auto",
      homeHeroTransientCollapsed: false,
      asciiBannerColor: randomAsciiBannerColor(),
      settingsSection: "general",
    },
    home: {
      workspace: "",
      providers: [],
      search: "",
      sort: "recent",
      hookFilter: "all",
      visible: 12,
      groups: [],
      syncGroups: [],
    },
    session: null,
    syncDetail: null,
    manager: {
      draft: null,
      preview: null,           // sessions: full filtered result (POST /manager/preview)
      workspacePreview: null,   // workspaces: full filtered result (POST /manager/workspaces)
      quickPreview: null,       // sessions: quick view (GET /manager/quick-preview)
      quickWorkspacePreview: null, // workspaces: quick view (GET /manager/quick-workspaces)
      isDefaultPreview: true,   // true = showing quick view, false = showing filtered result
      stats: null,
      statsLoading: false,
      statsRequestId: "",
      report: null,
      pendingItems: [],
      viewMode: "sessions",
      selectionMode: false,
      selectedItems: new Set(),          // encoded session item values
      selectedWorkspaceItems: new Set(), // encoded workspace item values
    },
    compression: {
      archives: [],
      providers: [],
      selectedArchive: null,
    },
    agents: {
      providers: [],
      providerDetails: {},
      providerDetailLoading: {},
      selectedProvider: "",
      settingResults: {},
      pendingSettings: {},
      hookDiagnostics: null,
      hookRuntimeSessions: [],
      hookDoctorReport: null,
      hookCleanupReport: null,
    },
    hooks: {
      overview: null,
      selectedProvider: "",
      providerDetail: null,
      diagnosisFilter: "attention",
      sessionDiagnosis: [],
    },
    updateCheck: {
      checking: false,
      result: null,
      error: "",
    },
    modal: null,
    toasts: [],
  };
}
