import { queryOptions, type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { detectAgent, getAgent, getMeta, getProviderCatalog, getProviderConfigView, listAgentsSummary, runProviderSetting, updateProviderSetting } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useAgentsSummary() {
  return useQuery({
    queryKey: queryKeys.agentsSummary,
    queryFn: listAgentsSummary,
  });
}

function agentOptions(provider: string) {
  return queryOptions({
    queryKey: queryKeys.agent(provider),
    queryFn: () => getAgent(provider),
  });
}

export function useAgent(provider: string | null) {
  return useQuery({
    ...agentOptions(provider || ""),
    enabled: !!provider,
  });
}

export function prefetchAgent(queryClient: QueryClient, provider: string) {
  return queryClient.prefetchQuery(agentOptions(provider));
}

export function useAgentsMeta() {
  return useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });
}

export function useAgentProviderCatalog(workspace?: string | null) {
  return useQuery({
    queryKey: queryKeys.providerCatalog(workspace),
    queryFn: () => getProviderCatalog(workspace),
  });
}

export function useDetectAgent() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (provider: string) => detectAgent(provider),
    onSuccess: (entry) => {
      queryClient.setQueryData(queryKeys.agent(entry.provider_id), entry);
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}

export function useUpdateProviderSetting() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, settingId, value }: { provider: string; settingId: string; value: unknown }) =>
      updateProviderSetting(provider, settingId, value),
    onSuccess: (_setting, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
      queryClient.invalidateQueries({ queryKey: queryKeys.meta });
    },
  });
}

export function useRunProviderSetting() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, settingId, workspace }: { provider: string; settingId: string; workspace?: string | null }) =>
      runProviderSetting(provider, settingId, workspace),
    onSuccess: (_output, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}

export function useProviderConfigView(provider: string | null, viewId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.providerConfigView(provider ?? "", viewId),
    queryFn: () => getProviderConfigView(provider!, viewId),
    enabled: enabled && !!provider,
  });
}
