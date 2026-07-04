import { useQuery } from "@tanstack/react-query";
import { getMeta, getProviderCatalog, getWorkspaceProviders, listProviders, listSessions, listSyncGroups } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useHomeData(workspace?: string | null, selectedProviders?: string[]) {
  const meta = useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });

  const selectedWorkspace = workspace || meta.data?.selected_workspace || undefined;
  const providerFilter = selectedProviders?.length ? selectedProviders.join(",") : undefined;
  const sessionParams = {
    all: false,
    details: true,
    limit: 6,
    workspace: selectedWorkspace,
    ...(providerFilter ? { provider: providerFilter } : {}),
  } as const;

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
    queryKey: queryKeys.sessions(sessionParams),
    queryFn: () => listSessions(sessionParams),
    enabled: !meta.isLoading && Boolean(selectedProviders?.length),
  });

  const syncGroups = useQuery({
    queryKey: queryKeys.syncGroups,
    queryFn: listSyncGroups,
  });

  return { meta, providers, catalog, workspaceProviders, sessions, syncGroups };
}
