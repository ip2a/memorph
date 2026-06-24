// 前端唯一 provider 消费门面：所有页面从这里取分类后的数据，按需自行渲染。

const ALIAS = {
  "claude-code": "claude",
  factory: "droid",
  "trae-cn": "traecn",
  "trae_cn": "traecn",
  "trae-gui": "trae_gui",
  "trae_gui": "trae_gui",
  "oh-my-pi": "omp",
  "oh_my_pi": "omp",
  "codybuddy-cn": "codybuddycn",
  "codybuddy_cn": "codybuddycn",
  "step-fun": "stepfun",
  "step_fun": "stepfun",
  "work-buddy": "workbuddy",
  "work_buddy": "workbuddy",
};

export function createProviders(state) {
  function normalizeId(raw) {
    const id = String(raw || "").trim().toLowerCase();
    return ALIAS[id] || id;
  }

  function list() {
    const providers = state.catalog?.providers || [];
    return [...providers];
  }

  function get(idOrEntry) {
    if (idOrEntry && typeof idOrEntry === "object") {
      const id = idOrEntry.provider_id || idOrEntry.id;
      return get(id);
    }
    const id = normalizeId(idOrEntry);
    return list().find((item) => item.provider_id === id) || null;
  }

  function all() {
    return list();
  }

  function visible() {
    return list().filter(
      (item) => !item.hidden_state?.global && !item.hidden_state?.workspace
    );
  }

  function byFilter(tag) {
    return list().filter((item) => (item.filter_tags || []).includes(tag));
  }

  function hasFilter(idOrEntry, tag) {
    const entry = get(idOrEntry);
    return entry ? (entry.filter_tags || []).includes(tag) : false;
  }

  function displayName(idOrEntry) {
    const entry = get(idOrEntry);
    if (entry) return entry.display_name;
    if (idOrEntry && typeof idOrEntry === "object") {
      return (
        idOrEntry.display_name ||
        idOrEntry.name ||
        idOrEntry.provider_id ||
        idOrEntry.id ||
        "Unknown"
      );
    }
    return String(idOrEntry || "Unknown");
  }

  function capabilitySet(idOrEntry) {
    return (
      get(idOrEntry)?.capability_set || {
        scan: false,
        import: false,
        export: false,
        delete: false,
        rename: false,
        resume: false,
      }
    );
  }

  function hiddenState(idOrEntry) {
    return (
      get(idOrEntry)?.hidden_state || {
        global: false,
        workspace: false,
      }
    );
  }

  function isHidden(idOrEntry) {
    const state = hiddenState(idOrEntry);
    return state.global || state.workspace;
  }

  function isHiddenGlobal(idOrEntry) {
    return hiddenState(idOrEntry).global;
  }

  function isHiddenWorkspace(idOrEntry) {
    return hiddenState(idOrEntry).workspace;
  }

  function isInstalled(idOrEntry) {
    return hasFilter(idOrEntry, "is_installed");
  }

  function hasSessions(idOrEntry) {
    return hasFilter(idOrEntry, "has_sessions");
  }

  function iconFor(idOrEntry) {
    const name = displayName(idOrEntry);
    return name ? name.charAt(0).toUpperCase() : "?";
  }

  return {
    normalizeId,
    all,
    visible,
    byFilter,
    hasFilter,
    displayName,
    capabilitySet,
    hiddenState,
    isHidden,
    isHiddenGlobal,
    isHiddenWorkspace,
    isInstalled,
    hasSessions,
    iconFor,
  };
}
