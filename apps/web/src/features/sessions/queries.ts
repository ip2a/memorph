import { useQuery } from "@tanstack/react-query";
import { getSession, listSessions } from "@/lib/api";
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
