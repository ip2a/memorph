import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getHookProviderOverview, getHooksOverview, getMeta, runHookProviderOperation } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useHooksOverview() {
  return useQuery({
    queryKey: queryKeys.hooks,
    queryFn: getHooksOverview,
  });
}

export function useHooksMeta() {
  return useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });
}

export function useHookProviderOverview(provider: string | null) {
  return useQuery({
    queryKey: queryKeys.hookProvider(provider || ""),
    queryFn: () => getHookProviderOverview(provider || ""),
    enabled: !!provider,
  });
}

export function useRunHookProviderOperation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, operation }: { provider: string; operation: string }) =>
      runHookProviderOperation(provider, operation),
    onSuccess: (_report, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.hooks });
      queryClient.invalidateQueries({ queryKey: queryKeys.hookProvider(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}
