import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getSkillContext,
  getSkillContextSummary,
  getSkillHealth,
  getSkillHealthSummary,
  getSkillGraph,
  getSkillConflicts,
  getSkillCoverage,
  getSkillCoverageSummary,
  getSkillCoverageEvidence,
  getSkillDetail,
  getSkillFilePreview,
  getSkillInvocations,
  getSkillTree,
  getSkills,
  getSkillStatsDaily,
  getSkillStatsBreakdown,
  getSkillStatsRanking,
  getSkillStatsSummary,
  installSkill,
  previewSkillPrune,
  executeSkillPrune,
  scanSkills,
  uninstallSkill,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type {
  SkillCatalogParams,
  SkillGraphParams,
  SkillStatsParams,
} from "@/lib/types";

export function useScanSkills() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      mode,
      workspace,
    }: {
      mode: "incremental" | "full";
      workspace?: string;
    }) => scanSkills(mode, workspace),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useSkills(params: SkillCatalogParams = {}) {
  return useQuery({
    queryKey: queryKeys.skills(params),
    queryFn: () => getSkills(params),
    placeholderData: (previous) => previous,
  });
}

export function useSkillContextSummary(provider?: string, baseline?: number) {
  return useQuery({
    queryKey: queryKeys.skillContextSummary(provider, baseline),
    queryFn: () => getSkillContextSummary(provider, baseline),
  });
}

export function useSkillContext(skillId: string | null, baseline?: number) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillContext(skillId, baseline)
      : ["skills", "context", "none"],
    queryFn: () => getSkillContext(skillId as string, baseline),
    enabled: Boolean(skillId),
  });
}

export function useSkillHealthSummary() {
  return useQuery({
    queryKey: queryKeys.skillHealthSummary,
    queryFn: getSkillHealthSummary,
  });
}

export function useSkillHealth(skillId: string | null) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillHealth(skillId)
      : ["skills", "health", "none"],
    queryFn: () => getSkillHealth(skillId as string),
    enabled: Boolean(skillId),
  });
}

export function useSkillConflicts(skillId?: string | null) {
  return useQuery({
    queryKey: queryKeys.skillConflicts(skillId ?? undefined),
    queryFn: () => getSkillConflicts(skillId ?? undefined),
  });
}

export function useSkillCoverageSummary(range: string) {
  return useQuery({
    queryKey: queryKeys.skillCoverageSummary(range),
    queryFn: () => getSkillCoverageSummary(range),
  });
}

export function useSkillCoverage(skillId: string | null, range: string) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillCoverage(skillId, range)
      : ["skills", "coverage", "none"],
    queryFn: () => getSkillCoverage(skillId as string, range),
    enabled: Boolean(skillId),
  });
}

export function useSkillCoverageEvidence(
  skillId: string | null,
  targetKey: string | null,
  page = 1,
) {
  return useQuery({
    queryKey:
      skillId && targetKey
        ? queryKeys.skillCoverageEvidence(skillId, targetKey, page)
        : ["skills", "coverage", "evidence", "none"],
    queryFn: () =>
      getSkillCoverageEvidence(skillId as string, targetKey as string, page),
    enabled: Boolean(skillId && targetKey),
  });
}

export function useSkillPrune(days: number) {
  return useQuery({
    queryKey: queryKeys.skillPrune(days),
    queryFn: () => previewSkillPrune(days),
  });
}
export function useExecuteSkillPrune() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      preview,
      installationIds,
    }: {
      preview: import("@/lib/types").SkillPrunePreview;
      installationIds: string[];
    }) => executeSkillPrune(preview, installationIds),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useSkillGraph(params: SkillGraphParams) {
  return useQuery({
    queryKey: queryKeys.skillGraph(params),
    queryFn: () => getSkillGraph(params),
  });
}

export function useSkillStats(params: SkillStatsParams) {
  return {
    summary: useQuery({
      queryKey: queryKeys.skillStatsSummary(params),
      queryFn: () => getSkillStatsSummary(params),
    }),
    daily: useQuery({
      queryKey: queryKeys.skillStatsDaily(params),
      queryFn: () => getSkillStatsDaily(params),
    }),
    ranking: useQuery({
      queryKey: queryKeys.skillStatsRanking(params),
      queryFn: () => getSkillStatsRanking(params),
    }),
    breakdown: useQuery({
      queryKey: queryKeys.skillStatsBreakdown(params),
      queryFn: () => getSkillStatsBreakdown(params),
    }),
  };
}

export function useSkillInvocations(
  skillId: string | null,
  params: SkillStatsParams,
) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillInvocations(skillId, params)
      : ["skills", "invocations", "none"],
    queryFn: () => getSkillInvocations(skillId as string, params),
    enabled: Boolean(skillId),
  });
}

export function useSkillDetail(skillId: string | null) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillDetail(skillId)
      : ["skills", "detail", "none"],
    queryFn: () => getSkillDetail(skillId as string),
    enabled: Boolean(skillId),
  });
}

export function useSkillTree(skillId: string | null) {
  return useQuery({
    queryKey: skillId
      ? queryKeys.skillTree(skillId)
      : ["skills", "tree", "none"],
    queryFn: () => getSkillTree(skillId as string),
    enabled: Boolean(skillId),
  });
}

export function useSkillFilePreview(
  skillId: string | null,
  path: string | null,
  provider?: string,
) {
  return useQuery({
    queryKey:
      skillId && path
        ? queryKeys.skillFile(skillId, path, provider)
        : ["skills", "file", "none"],
    queryFn: () =>
      getSkillFilePreview(skillId as string, path as string, provider),
    enabled: Boolean(skillId && path),
  });
}
export function useInstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: installSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useUninstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: uninstallSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}
