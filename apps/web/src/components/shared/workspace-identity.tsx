import { PathText } from "@/components/shared/path-text";
import { workspaceName } from "@/components/shared/workspace-name";
import { cn } from "@/lib/utils";

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
  fallbackTitle = "No workspace",
  className,
  labelClassName,
  titleClassName,
  pathClassName,
}: WorkspaceIdentityProps) {
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <span className={cn("text-muted-foreground font-mono text-xs uppercase", labelClassName)}>Workspace</span>
      <strong className={cn("text-lg font-semibold leading-tight", titleClassName)}>{workspaceName(workspace, fallbackTitle)}</strong>
      <PathText value={workspace} className={pathClassName} />
    </div>
  );
}
