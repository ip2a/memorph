import { useQuery } from "@tanstack/react-query";
import { getSyncGroup, listSyncGroups } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useSyncGroups() {
  return useQuery({
    queryKey: queryKeys.syncGroups,
    queryFn: listSyncGroups,
  });
}

export function useSyncGroup(groupId: string) {
  return useQuery({
    queryKey: queryKeys.syncGroup(groupId),
    queryFn: () => getSyncGroup(groupId),
    enabled: Boolean(groupId),
  });
}
