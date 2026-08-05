import { queryOptions, type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { deleteDetectedHook, detectAgent, getAgent, getMeta, getProviderCatalog, getProviderConfigView, listAgentsSummary, listDetectedHooks, removeProviderConfigEntry, runHookProviderOperation, runProviderSetting, updateProviderSetting } from "@/lib/api";
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

export function useRemoveProviderConfigEntry() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      provider,
      viewId,
      entryId,
      expectedFingerprint,
    }: {
      provider: string;
      viewId: string;
      entryId: string;
      expectedFingerprint: string;
    }) => removeProviderConfigEntry(provider, viewId, entryId, expectedFingerprint),
    onSuccess: async (_result, variables) => {
      await queryClient.invalidateQueries({
        queryKey: queryKeys.providerConfigView(variables.provider, variables.viewId),
      });
      await queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      await queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}

export function useRunAgentHookOperation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, operation }: { provider: string; operation: string }) =>
      runHookProviderOperation(provider, operation),
    onSuccess: (_report, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentHooks(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}

export function useDeleteAgentDetectedHook() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      provider,
      event,
      index,
      fingerprint,
    }: {
      provider: string;
      event: string;
      index: number;
      fingerprint: string;
    }) => deleteDetectedHook(provider, event, index, fingerprint),
    onSuccess: (_report, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agent(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentHooks(variables.provider) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
    },
  });
}

export function useAgentDetectedHooks(provider: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.agentHooks(provider || ""),
    queryFn: () => listDetectedHooks(provider || ""),
    enabled: enabled && !!provider,
  });
}
