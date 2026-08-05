import { useQuery } from "@tanstack/react-query";
import { getMeta, getProviderCatalog, getWorkspaceProviders, listProviders, listSessions, listSyncGroups } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { SessionListSort } from "@/lib/types";

type HomeSessionOptions = {
  sort?: SessionListSort;
  sessionLimit?: number;
};

function isFeedBusy(kind?: string) {
  return kind === "cold_scanning" || kind === "warming";
}

export function useHomeData(
  workspace?: string | null,
  selectedProviders?: string[],
  sessionOptions: HomeSessionOptions = {},
) {
  const meta = useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });

  const selectedWorkspace = workspace || meta.data?.selected_workspace || undefined;
  const providerFilter = selectedProviders?.length ? selectedProviders.join(",") : undefined;
  const sessionLimit = Math.max(
    1,
    Math.min(
      200,
      Number(sessionOptions.sessionLimit ?? (meta.data?.settings.sessions_per_provider || 6)),
    ),
  );
  const sessionParams = {
    all: false,
    fields: "with_stats" as const,
    limit: sessionLimit,
    workspace: selectedWorkspace,
    sort: sessionOptions.sort ?? "recent",
    refresh: false,
    ...(providerFilter ? { provider: providerFilter } : {}),
  };

  const providers = useQuery({
    queryKey: queryKeys.providers,
    queryFn: listProviders,
  });

  const catalog = useQuery({
    queryKey: queryKeys.providerCatalog(selectedWorkspace),
    queryFn: () => getProviderCatalog(selectedWorkspace),
    enabled: !meta.isLoading,
  });

  const workspaceProviders = useQuery({
    queryKey: queryKeys.workspaceProviders(selectedWorkspace || ""),
    queryFn: () => getWorkspaceProviders(selectedWorkspace || ""),
    enabled: Boolean(selectedWorkspace) && !meta.isLoading,
  });

  const sessions = useQuery({
    queryKey: queryKeys.sessionPage(sessionParams),
    queryFn: () => listSessions(sessionParams),
    enabled: !meta.isLoading && Boolean(selectedProviders?.length),
    placeholderData: (previous) => previous,
    // The workspace feed returns immediately and scans providers in the
    // background. Poll while it reports a non-ready state.
    refetchInterval: (query) =>
      isFeedBusy(query.state.data?.feed_state?.kind) ? 2000 : false,
  });

  const syncGroups = useQuery({
    queryKey: queryKeys.syncGroups,
    queryFn: listSyncGroups,
  });

  return { meta, providers, catalog, workspaceProviders, sessions, syncGroups, sessionParams };
}
