import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cleanupArtifacts, getBackup, inspectArtifacts, listBackups, restoreBackup } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ArtifactCleanupRequest, BackupQueryParams } from "@/lib/types";

export function useArtifactInspection() {
  return useQuery({
    queryKey: queryKeys.artifactInspection,
    queryFn: inspectArtifacts,
  });
}

export function useCleanupArtifacts() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (payload: ArtifactCleanupRequest) => cleanupArtifacts(payload),
    onSuccess: (report) => {
      if (report.applied) {
        queryClient.invalidateQueries({ queryKey: queryKeys.artifacts });
      }
    },
  });
}

export function useBackups(params: BackupQueryParams = {}) {
  return useQuery({
    queryKey: queryKeys.backups(params),
    queryFn: () => listBackups(params),
  });
}

export function useBackup(backupId: string | null) {
  return useQuery({
    queryKey: queryKeys.backup(backupId || ""),
    queryFn: () => getBackup(backupId || ""),
    enabled: !!backupId,
  });
}

export function useRestoreBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (backupId: string) => restoreBackup(backupId),
    onSuccess: (_restore, backupId) => {
      queryClient.invalidateQueries({ queryKey: ["backups"] });
      queryClient.invalidateQueries({ queryKey: queryKeys.backup(backupId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.artifactInspection });
    },
  });
}
