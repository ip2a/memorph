export function parseRoute(pathname) {
  if (pathname === "/agents" || pathname === "/tools") return { name: "agents" };
  if (pathname === "/hooks") return { name: "hooks" };
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
