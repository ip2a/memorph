import { useQuery } from "@tanstack/react-query";
import { getSession, getSessionActivity, listSessions } from "@/lib/api";
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
  });
}

export function useSessionActivity(provider: string, sessionId: string) {
  return useQuery({
    queryKey: queryKeys.sessionActivity(provider, sessionId),
    queryFn: () => getSessionActivity(provider, sessionId),
    enabled: provider.length > 0 && sessionId.length > 0,
  });
}
