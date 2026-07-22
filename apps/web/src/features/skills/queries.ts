import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  deleteSkillRelation,
  getSkillAnalysis,
  getSkillDetail,
  getSkillFilePreview,
  getSkillInvocations,
  getSkillRelationCandidates,
  getSkillRelations,
  getSkillTree,
  getSkills,
  getSkillStatsDaily,
  getSkillStatsRanking,
  getSkillStatsSummary,
  ignoreSkillRelationCandidate,
  installSkill,
  saveSkillGroup,
  saveSkillRelation,
  uninstallSkill,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { SkillCatalogParams, SkillStatsParams } from "@/lib/types";

export function useSkills(params: SkillCatalogParams = {}) {
  return useQuery({
    queryKey: queryKeys.skills(params),
    queryFn: () => getSkills(params),
    placeholderData: (previous) => previous,
  });
}

export function useSkillAnalysis() {
  return useQuery({
    queryKey: queryKeys.skillAnalysis,
    queryFn: () => getSkillAnalysis(),
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
export function useSkillRelations() {
  return useQuery({
    queryKey: queryKeys.skillRelations,
    queryFn: getSkillRelations,
  });
}

export function useSkillRelationCandidates() {
  return useQuery({
    queryKey: queryKeys.skillRelationCandidates,
    queryFn: getSkillRelationCandidates,
  });
}

export function useSaveSkillGroup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: saveSkillGroup,
    onSuccess: (config) => {
      queryClient.setQueryData(queryKeys.skillRelations, config);
      queryClient.invalidateQueries({
        queryKey: queryKeys.skillRelationCandidates,
      });
    },
  });
}
export function useSaveSkillRelation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: saveSkillRelation,
    onSuccess: (config) => {
      queryClient.setQueryData(queryKeys.skillRelations, config);
      queryClient.invalidateQueries({
        queryKey: queryKeys.skillRelationCandidates,
      });
    },
  });
}

export function useDeleteSkillRelation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteSkillRelation,
    onSuccess: (config) =>
      queryClient.setQueryData(queryKeys.skillRelations, config),
  });
}

export function useIgnoreSkillRelationCandidate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ignoreSkillRelationCandidate,
    onSuccess: (config) => {
      queryClient.setQueryData(queryKeys.skillRelations, config);
      queryClient.invalidateQueries({
        queryKey: queryKeys.skillRelationCandidates,
      });
    },
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
