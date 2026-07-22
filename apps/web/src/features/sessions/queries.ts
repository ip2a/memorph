import { useMutation, useQuery, useQueryClient, keepPreviousData } from "@tanstack/react-query";
import { getSession, getSessionActivity, listSessions, refreshSessionStaleness, reprojectStaleSessions } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { SessionDetailParams, SessionListParams } from "@/lib/types";

export function useSessions(params: SessionListParams) {
  return useQuery({
    queryKey: queryKeys.sessions(params),
    queryFn: () => listSessions(params),
  });
}

export function useSession(provider: string, sessionId: string, params: SessionDetailParams = {}) {
  return useQuery({
    queryKey: queryKeys.session(provider, sessionId, params),
    queryFn: () => getSession(provider, sessionId, params),
    enabled: provider.length > 0 && sessionId.length > 0,
    placeholderData: keepPreviousData,
  });
}

export function useSessionActivity(provider: string, sessionId: string) {
  return useQuery({
    queryKey: queryKeys.sessionActivity(provider, sessionId),
    queryFn: () => getSessionActivity(provider, sessionId),
    enabled: provider.length > 0 && sessionId.length > 0,
  });
}

export function useRefreshSessionStaleness() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: refreshSessionStaleness,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
  });
}

export function useReprojectStaleSessions() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (provider?: string | null) => reprojectStaleSessions(provider),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
  });
}
