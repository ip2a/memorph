import { ASCII_BANNER_COLORS } from "./constants.js";
import { parseRoute } from "./router.js";

export function randomAsciiBannerColor() {
  return ASCII_BANNER_COLORS[Math.floor(Math.random() * ASCII_BANNER_COLORS.length)];
}

export function createState() {
  return {
    meta: null,
    route: parseRoute(window.location.pathname),
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
      hookDiagnostics: null,
      hookRuntimeSessions: [],
      hookPendingRequests: [],
      hookPolicy: null,
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
