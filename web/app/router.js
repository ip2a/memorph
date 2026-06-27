const ROUTE_PAGE_META = {
  home: {
    titleKey: "",
    shell: "default",
    scrollClass: "",
    fallbackPath: "/",
  },
  session: {
    titleKey: "details",
    shell: "default",
    scrollClass: "page-scroll",
    fallbackPath: "/",
  },
  "sync-list": {
    titleKey: "syncTitle",
    shell: "default",
    scrollClass: "page-scroll manager-page-scroll",
    fallbackPath: "/",
  },
  "sync-detail": {
    titleKey: "syncTitle",
    shell: "default",
    scrollClass: "page-scroll",
    fallbackPath: "/sync",
  },
  manager: {
    titleKey: "managerTitle",
    shell: "manager",
    scrollClass: "page-scroll manager-page-scroll",
    fallbackPath: "/",
  },
  compression: {
    titleKey: "compressionTitle",
    shell: "manager",
    scrollClass: "page-scroll manager-page-scroll",
    fallbackPath: "/manager",
  },
  hooks: {
    titleKey: "hooksTitle",
    shell: "manager",
    scrollClass: "page-scroll manager-page-scroll",
    fallbackPath: "/",
  },
  agents: {
    titleKey: "agentManagementTitle",
    shell: "manager",
    scrollClass: "page-scroll manager-page-scroll",
    fallbackPath: "/",
  },
};

export function parseRoute(pathname, searchParams = null) {
  if (pathname === "/agents" || pathname === "/tools") return { name: "agents" };
  if (pathname === "/hooks") return { name: "hooks" };
  if (pathname === "/manager") {
    const params = searchParams || new URLSearchParams();
    return {
      name: "manager",
      provider: params.get("provider") || undefined,
      workspace: params.get("workspace") || undefined,
      view: params.get("view") || undefined,
    };
  }
  if (pathname === "/compression") return { name: "compression" };
  if (pathname === "/sync") return { name: "sync-list" };
  const sessionMatch = pathname.match(/^\/sessions\/([^/]+)\/([^/]+)$/);
  if (sessionMatch) {
    return {
      name: "session",
      provider: decodeURIComponent(sessionMatch[1]),
      sessionId: decodeURIComponent(sessionMatch[2]),
    };
  }
  const syncMatch = pathname.match(/^\/sync\/([^/]+)$/);
  if (syncMatch) {
    return {
      name: "sync-detail",
      groupId: decodeURIComponent(syncMatch[1]),
    };
  }
  return { name: "home" };
}

export function routePageMeta(route) {
  return ROUTE_PAGE_META[route?.name] || ROUTE_PAGE_META.home;
}

export function routeTitleKey(route) {
  return routePageMeta(route).titleKey;
}

export function routeBackTarget(route) {
  return routePageMeta(route).fallbackPath;
}

export function routeShellClass(route) {
  return routePageMeta(route).shell === "manager" ? "manager-shell" : "";
}

export function routeScrollClass(route) {
  return routePageMeta(route).scrollClass || "page-scroll";
}
