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
