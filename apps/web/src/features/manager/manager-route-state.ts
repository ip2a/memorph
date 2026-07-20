import type { ManagerFilter } from "@/lib/types";
import {
  clampManagerPageSize,
  DEFAULT_MANAGER_PAGE_SIZE,
  readManagerPage,
  type ManagerPageSize,
} from "@/features/manager/manager-pagination";

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
  page: number;
  pageSize: ManagerPageSize;
};

export type ManagerRequest = {
  listFilter: ManagerFilter;
  statsFilter: ManagerFilter;
  enabled: boolean;
  workspace: string | null;
  page: number;
  pageSize: ManagerPageSize;
};

function baseManagerFilter(
  route: ManagerRouteState,
  workspace: string | null,
): ManagerFilter {
  return {
    providers: route.providers,
    workspace: workspace ?? undefined,
    sort: route.sort,
  };
}

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
    page: readManagerPage(searchParams),
    pageSize: clampManagerPageSize(searchParams.get("pageSize")),
  };
}

export function resolveManagerRequest(
  route: ManagerRouteState,
  currentWorkspace: string | null,
): ManagerRequest {
  const workspace = route.workspace || (route.scope === "current" ? currentWorkspace : null);
  const pageSize = route.pageSize || DEFAULT_MANAGER_PAGE_SIZE;
  const page = route.page;
  const search = route.search.trim();

  const statsFilter = baseManagerFilter(route, workspace);
  const listFilter: ManagerFilter = {
    ...baseManagerFilter(route, workspace),
    search: search || undefined,
    limit: pageSize,
    offset: (page - 1) * pageSize,
  };

  return {
    listFilter,
    statsFilter,
    enabled: route.scope === "all" || Boolean(workspace),
    workspace,
    page,
    pageSize,
  };
}

export function resetManagerPageParam(next: URLSearchParams): void {
  next.delete("page");
}
