import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getSkills, installSkill, uninstallSkill } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useSkills() {
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: getSkills,
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
