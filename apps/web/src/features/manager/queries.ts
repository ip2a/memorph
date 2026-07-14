import { useQuery } from "@tanstack/react-query";
import {
  getMeta,
  getManagerPreview,
  getManagerQuickPreview,
  getManagerQuickWorkspaces,
  getManagerStats,
  getManagerWorkspaces,
  listProviders,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ManagerFilter } from "@/lib/types";

export function useManagerProviders() {
  return useQuery({
    queryKey: queryKeys.providers,
    queryFn: listProviders,
  });
}

export function useManagerMeta() {
  return useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });
}

export function useManagerQuickPreview(providers: string[]) {
  return useQuery({
    queryKey: queryKeys.managerQuick(providers),
    queryFn: () => getManagerQuickPreview(providers),
  });
}

export function useManagerQuickWorkspaces(providers: string[]) {
  return useQuery({
    queryKey: queryKeys.managerQuickWorkspaces(providers),
    queryFn: () => getManagerQuickWorkspaces(providers),
  });
}

export function useManagerStats(filter: ManagerFilter, options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.managerStats(filter),
    queryFn: () => getManagerStats(filter),
    enabled: options?.enabled ?? true,
  });
}

export function useManagerPreview(filter: ManagerFilter, options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.manager("sessions", filter),
    queryFn: () => getManagerPreview(filter),
    enabled: options?.enabled ?? true,
  });
}

export function useManagerWorkspaces(filter: ManagerFilter, options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.manager("workspaces", filter),
    queryFn: () => getManagerWorkspaces(filter),
    enabled: options?.enabled ?? true,
  });
}
