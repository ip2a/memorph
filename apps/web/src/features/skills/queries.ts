import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getSkillDetail, getSkillFilePreview, getSkillTree, getSkills, installSkill, uninstallSkill } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useSkills() {
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: getSkills,
  });
}

export function useSkillDetail(skillId: string | null) {
  return useQuery({
    queryKey: skillId ? queryKeys.skillDetail(skillId) : ["skills", "detail", "none"],
    queryFn: () => getSkillDetail(skillId as string),
    enabled: Boolean(skillId),
  });
}

export function useSkillTree(skillId: string | null) {
  return useQuery({
    queryKey: skillId ? queryKeys.skillTree(skillId) : ["skills", "tree", "none"],
    queryFn: () => getSkillTree(skillId as string),
    enabled: Boolean(skillId),
  });
}

export function useSkillFilePreview(skillId: string | null, path: string | null, provider?: string) {
  return useQuery({
    queryKey: skillId && path ? queryKeys.skillFile(skillId, path, provider) : ["skills", "file", "none"],
    queryFn: () => getSkillFilePreview(skillId as string, path as string, provider),
    enabled: Boolean(skillId && path),
  });
}
export function useInstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: installSkill,
    onSuccess: (overview) =>
      queryClient.setQueryData(queryKeys.skills, overview),
  });
}

export function useUninstallSkill() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: uninstallSkill,
    onSuccess: (overview) =>
      queryClient.setQueryData(queryKeys.skills, overview),
  });
}
