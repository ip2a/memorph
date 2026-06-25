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
      preview: null,
      workspacePreview: null,
      stats: null,
      statsLoading: false,
      statsRequestId: "",
      report: null,
      pendingItems: [],
      viewMode: "sessions",
    },
    compression: {
      archives: [],
      providers: [],
      selectedArchive: null,
    },
    agents: {
      providers: [],
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
