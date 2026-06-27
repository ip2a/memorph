export function createNavigation({
  parseRoute,
  getRoute,
  setRoute,
  fallbackPath,
  onBeforeNavigation,
  onHomeRoute,
  loadRoute,
  readPageState,
  applyPageState,
}) {
  let restoreAttempts = 0;
  let pendingPageState = null;
  let saveTimer = 0;

  function currentPath() {
    return window.location.pathname + window.location.search;
  }

  function parseCurrentRoute() {
    return parseRoute(window.location.pathname, new URLSearchParams(window.location.search));
  }

  function entryState(patch = {}) {
    return { ...(history.state || {}), app: "memorph", ...patch };
  }

  function saveCurrentPageState() {
    const pageState = readPageState(getRoute());
    history.replaceState(entryState({ pageState }), "", currentPath());
  }

  function schedulePageStateSave() {
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      saveTimer = 0;
      saveCurrentPageState();
    }, 120);
  }

  function markPageStateForRestore() {
    pendingPageState = history.state?.pageState || null;
    restoreAttempts = 4;
    applyPageState(getRoute(), pendingPageState);
  }

  function setCurrentRoute(route, refreshHomeChrome) {
    setRoute(route);
    if (refreshHomeChrome && route.name === "home") {
      onHomeRoute();
    }
    markPageStateForRestore();
  }

  function loadCurrentRoute(refreshHomeChrome = false) {
    setCurrentRoute(parseCurrentRoute(), refreshHomeChrome);
    void loadRoute();
  }

  function init() {
    if ("scrollRestoration" in history) {
      history.scrollRestoration = "manual";
    }
    history.replaceState(
      entryState({
        from: history.state?.from || null,
        pageState: history.state?.pageState || null,
      }),
      "",
      currentPath()
    );
    window.addEventListener("popstate", () => loadCurrentRoute(false));
    window.addEventListener("pagehide", saveCurrentPageState);
    window.addEventListener("scroll", schedulePageStateSave, true);
  }

  function navigate(path) {
    const url = new URL(path, window.location.href);
    const previousPath = currentPath();
    const samePath = window.location.pathname === url.pathname && window.location.search === url.search;
    saveCurrentPageState();
    onBeforeNavigation();
    if (!samePath) {
      history.pushState({ app: "memorph", from: previousPath, pageState: null }, "", path);
    }
    setCurrentRoute(parseRoute(url.pathname, url.searchParams), true);
    void loadRoute();
  }

  function replacePath(path) {
    const url = new URL(path, window.location.href);
    history.replaceState(entryState(), "", path);
    setRoute(parseRoute(url.pathname, url.searchParams));
  }

  function replaceNavigate(path) {
    const url = new URL(path, window.location.href);
    onBeforeNavigation();
    history.replaceState({ app: "memorph", from: null, pageState: null }, "", path);
    setCurrentRoute(parseRoute(url.pathname, url.searchParams), true);
    void loadRoute();
  }

  function goBack() {
    const from = history.state?.from;
    saveCurrentPageState();
    onBeforeNavigation();
    if (from && from !== currentPath()) {
      history.back();
      return;
    }
    replaceNavigate(fallbackPath(getRoute()));
  }

  function restorePageState() {
    if (restoreAttempts <= 0) return;
    restoreAttempts -= 1;
    const route = getRoute();
    const pageState = pendingPageState;
    window.requestAnimationFrame(() => applyPageState(route, pageState));
  }

  return {
    currentPath,
    goBack,
    init,
    navigate,
    replaceNavigate,
    replacePath,
    restorePageState,
    saveCurrentPageState,
  };
}
