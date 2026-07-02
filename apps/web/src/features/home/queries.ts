import { useQuery } from "@tanstack/react-query";
import { getMeta, listProviders, listSessions, listSyncGroups } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useHomeData(workspace?: string | null) {
  const meta = useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });

  const selectedWorkspace = workspace || meta.data?.selected_workspace || undefined;
  const sessionParams = { all: true, details: true, limit: 6, workspace: selectedWorkspace } as const;

  const providers = useQuery({
    queryKey: queryKeys.providers,
    queryFn: listProviders,
  });

  const sessions = useQuery({
    queryKey: queryKeys.sessions(sessionParams),
    queryFn: () => listSessions(sessionParams),
    enabled: !meta.isLoading,
  });

  const syncGroups = useQuery({
    queryKey: queryKeys.syncGroups,
    queryFn: listSyncGroups,
  });

  return { meta, providers, sessions, syncGroups };
}
