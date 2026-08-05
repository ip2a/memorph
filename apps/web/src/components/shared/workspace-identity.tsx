import { PathText } from "@/components/shared/path-text";
import { workspaceName } from "@/components/shared/workspace-name";
import { cn } from "@/lib/utils";
import { useI18n } from "@/lib/i18n-context";

type WorkspaceIdentityProps = {
  workspace: string | null | undefined;
  fallbackTitle?: string;
  className?: string;
  labelClassName?: string;
  titleClassName?: string;
  pathClassName?: string;
};

export function WorkspaceIdentity({
  workspace,
  fallbackTitle,
  className,
  labelClassName,
  titleClassName,
  pathClassName,
}: WorkspaceIdentityProps) {
  const { t } = useI18n();
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <span className={cn("text-muted-foreground font-mono text-xs uppercase", labelClassName)}>{t("workspace")}</span>
      <strong className={cn("text-lg font-semibold leading-tight", titleClassName)}>{workspaceName(workspace, fallbackTitle ?? t("workspaceNoWorkspace"))}</strong>
      <PathText value={workspace} className={pathClassName} />
    </div>
  );
}
