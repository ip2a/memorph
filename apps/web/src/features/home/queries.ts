import { useQuery } from "@tanstack/react-query";
import { getMeta, getProviderCatalog, getSessionFeedRevision, getWorkspaceProviders, listProviders, listSessions, listSyncGroups } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { SessionListSort } from "@/lib/types";

type HomeSessionOptions = {
  sort?: SessionListSort;
  sessionLimit?: number;
};

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

  // Two-level feed sync (scan-mechanism v2): a cheap revision poll runs on a
  // cadence that tightens while a scan is busy and relaxes once the workspace
  // settles; the full session list refetches only when the revision moves.
  // refetchIntervalInBackground is off so a hidden tab does not poll.
  const sessionsEnabled = !meta.isLoading && Boolean(selectedProviders?.length);
  const feedRevision = useQuery({
    queryKey: queryKeys.sessionFeedRevision(selectedWorkspace || ""),
    queryFn: () => getSessionFeedRevision(selectedWorkspace || ""),
    enabled: Boolean(selectedWorkspace) && sessionsEnabled,
    refetchInterval: (query) => (query.state.data?.busy ? 2000 : 15000),
    refetchIntervalInBackground: false,
  });

  const sessions = useQuery({
    queryKey: queryKeys.homeSessionPage(sessionParams, feedRevision.data?.revision ?? 0),
    queryFn: () => listSessions(sessionParams),
    enabled: sessionsEnabled,
    placeholderData: (previous) => previous,
  });

  const syncGroups = useQuery({
    queryKey: queryKeys.syncGroups,
    queryFn: listSyncGroups,
  });

  return { meta, providers, catalog, workspaceProviders, sessions, syncGroups, sessionParams };
}
