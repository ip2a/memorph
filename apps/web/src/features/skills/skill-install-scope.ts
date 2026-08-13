import type { SkillMutation } from "@/lib/types";

export type SkillInstallScope = {
  usedBy: string;
  scopeKind: "global" | "project";
  workspaceDir?: string;
};

export function skillInstallScopeKey(scope: SkillInstallScope) {
  return `${scope.usedBy}:${scope.scopeKind}`;
}

export function skillInstallScopeFromMutation(
  mutation: SkillMutation,
): SkillInstallScope {
  return {
    usedBy: mutation.used_by,
    scopeKind: mutation.scope_kind ?? "global",
    workspaceDir: mutation.workspace_dir,
  };
}

export function skillInstallScopeToMutation(
  scope: SkillInstallScope,
  skillId: string,
  sourceUsedBy?: string,
): SkillMutation {
  return {
    skill_id: skillId,
    used_by: scope.usedBy,
    source_used_by: sourceUsedBy,
    scope_kind: scope.scopeKind,
    workspace_dir:
      scope.scopeKind === "project" ? scope.workspaceDir : undefined,
  };
}
