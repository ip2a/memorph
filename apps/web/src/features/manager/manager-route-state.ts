import type { ManagerFilter } from "@/lib/types";

export type ManagerView = "sessions" | "workspaces";
export type ManagerScope = "current" | "all";
export type ManagerSort = "recent" | "size" | "title" | "sessions";

export type ManagerRouteState = {
  view: ManagerView;
  scope: ManagerScope;
  workspace: string | null;
  providers: string[];
  search: string;
  sort: ManagerSort;
};

export function readManagerRouteState(searchParams: URLSearchParams): ManagerRouteState {
  const providers = Array.from(
    new Set(
      (searchParams.get("providers") ?? "")
        .split(",")
        .map((provider) => provider.trim())
        .filter(Boolean),
    ),
  );
  const view = searchParams.get("view") === "workspaces" ? "workspaces" : "sessions";
  const sort = searchParams.get("sort");
  const normalizedSort =
    sort === "size" || sort === "title" || (view === "workspaces" && sort === "sessions")
      ? sort
      : "recent";

  return {
    view,
    scope: searchParams.get("scope") === "all" ? "all" : "current",
    workspace: searchParams.get("workspace") || null,
    providers,
    search: searchParams.get("q") ?? "",
    sort: normalizedSort,
  };
}

export function resolveManagerRequest(
  route: ManagerRouteState,
  currentWorkspace: string | null,
): { filter: ManagerFilter; enabled: boolean; workspace: string | null } {
  const workspace = route.workspace || (route.scope === "current" ? currentWorkspace : null);

  return {
    filter: {
      providers: route.providers,
      workspace: workspace ?? undefined,
      sort: route.sort,
      limit: 100,
    },
    enabled: route.scope === "all" || Boolean(workspace),
    workspace,
  };
}
