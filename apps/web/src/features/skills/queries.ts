import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  analyzeSkills,
  getSkillAnalysisOperation,
  getCurrentSkillAnalysis,
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
  scanSkills,
  uninstallSkill,
  deleteSkill,
  disableSkill,
  enableSkill,
  listDisabledSkills,
  consolidateSkill,
  removeSymlinksSkill,
  getSkillGroupInstallations,
  updateSkillFile,
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
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
      window.setTimeout(() => {
        void queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
      }, 1000);
    },
  });
}

export function useAnalyzeSkills() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (mode: "incremental" | "full" = "incremental") =>
      analyzeSkills(mode),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useSkillAnalysisOperation(operationId: string | null) {
  return useQuery({
    queryKey: ["skills", "analysis-operation", operationId],
    queryFn: () => getSkillAnalysisOperation(operationId as string),
    enabled: Boolean(operationId),
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      return status === "queued" || status === "running" ? 1500 : false;
    },
  });
}

export function useCurrentSkillAnalysis() {
  return useQuery({
    queryKey: ["skills", "analysis-operation", "current"],
    queryFn: getCurrentSkillAnalysis,
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      return status === "queued" || status === "running" ? 1500 : false;
    },
  });
}

export function useSkills(params: SkillCatalogParams = {}) {
  return useQuery({
    queryKey: queryKeys.skills(params),
    queryFn: () => getSkills(params),
    placeholderData: (previous) => previous,
    // While the catalog signals it needs a scan (background scan in flight),
    // poll every 2s so the list refreshes as soon as data lands.
    refetchInterval: (query) =>
      query.state.data?.needs_scan ? 2000 : false,
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
  usedBy?: string,
) {
  return useQuery({
    queryKey:
      skillId && path
        ? queryKeys.skillFile(skillId, path, usedBy)
        : ["skills", "file", "none"],
    queryFn: () =>
      getSkillFilePreview(skillId as string, path as string, usedBy),
    enabled: Boolean(skillId && path),
  });
}

export function useUpdateSkillFile() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      skillId,
      path,
      content,
      usedBy,
    }: {
      skillId: string;
      path: string;
      content: string;
      usedBy?: string;
    }) => updateSkillFile(skillId, path, content, usedBy),
    onSuccess: (preview, variables) => {
      queryClient.setQueryData(
        queryKeys.skillFile(variables.skillId, variables.path, variables.usedBy),
        preview,
      );
      void queryClient.invalidateQueries({
        queryKey: queryKeys.skillTree(variables.skillId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
    },
  });
}
export function useInstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: installSkill,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
    },
  });
}

export function useUninstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: uninstallSkill,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
    },
  });
}

export function useDeleteSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useDisableSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: disableSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useEnableSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ usedBy, directory }: { usedBy: string; directory: string }) =>
      enableSkill(usedBy, directory),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useConsolidateSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: consolidateSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useRemoveSymlinksSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: removeSymlinksSkill,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot }),
  });
}

export function useDisabledSkills() {
  return useQuery({
    queryKey: queryKeys.skillsDisabled,
    queryFn: listDisabledSkills,
  });
}

export function useSkillGroupInstallations(sourceId: string | null) {
  return useQuery({
    queryKey: sourceId
      ? queryKeys.skillGroupInstallations(sourceId)
      : ["skills", "group-installations", "none"],
    queryFn: () => getSkillGroupInstallations(sourceId as string),
    enabled: Boolean(sourceId),
  });
}
