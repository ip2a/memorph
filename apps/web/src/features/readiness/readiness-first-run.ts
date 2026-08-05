export function readinessFirstRunKey(workspace: string | null) {
  return `memorph.readiness.first-run-seen:${workspace ?? "__default__"}`;
}

export function hasSeenReadinessFirstRun(workspace: string | null) {
  if (typeof window === "undefined") return true;
  return Boolean(window.localStorage.getItem(readinessFirstRunKey(workspace)));
}

export function markReadinessFirstRunSeen(workspace: string | null) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(readinessFirstRunKey(workspace), "1");
}
