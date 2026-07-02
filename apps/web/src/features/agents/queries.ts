import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { detectAgent, getAgent, getMeta, listAgentsSummary, runProviderSetting, updateProviderSetting } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useAgentsSummary() {
  return useQuery({
    queryKey: queryKeys.agentsSummary,
    queryFn: listAgentsSummary,
  });
}

export function useAgent(provider: string | null) {
  return useQuery({
    queryKey: queryKeys.agent(provider || ""),
    queryFn: () => getAgent(provider || ""),
    enabled: !!provider,
  });
}

export function useAgentsMeta() {
  return useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
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
