import { useMutation } from "@tanstack/react-query";
import { useQuery } from "@tanstack/react-query";
import { applyCompression, getCompressionArchive, listCompressionArchives, listCompressionProviders, restoreCompressionArchive } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ApplyCompressionPayload, CompressionArchivesParams, RestoreCompressionPayload } from "@/lib/types";

export function useCompressionArchives(params: CompressionArchivesParams = {}) {
  return useQuery({
    queryKey: queryKeys.compression(params),
    queryFn: () => listCompressionArchives(params),
  });
}

export function useCompressionArchive(archiveRef: string) {
  return useQuery({
    queryKey: queryKeys.compressionArchive(archiveRef),
    queryFn: () => getCompressionArchive(archiveRef),
    enabled: archiveRef.length > 0,
  });
}

export function useCompressionProviders() {
  return useQuery({
    queryKey: queryKeys.compressionProviders,
    queryFn: listCompressionProviders,
  });
}

export function useApplyCompression() {
  return useMutation({
    mutationFn: (payload: ApplyCompressionPayload) => applyCompression(payload),
  });
}

export function useRestoreCompressionArchive() {
  return useMutation({
    mutationFn: (payload: RestoreCompressionPayload) => restoreCompressionArchive(payload),
  });
}
