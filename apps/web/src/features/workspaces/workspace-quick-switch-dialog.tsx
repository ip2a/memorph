import { useEffect, useState } from "react";
import { PathText } from "@/components/shared/path-text";
import { workspaceName } from "@/components/shared/workspace-name";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { useSwitchWorkspace } from "@/features/workspaces/use-switch-workspace";
import { useUiStore } from "@/stores/ui-store";

export function WorkspaceQuickSwitchDialog() {
  const quickSwitch = useUiStore((state) => state.workspaceQuickSwitch);
  const closeWorkspaceQuickSwitch = useUiStore(
    (state) => state.closeWorkspaceQuickSwitch,
  );
  const [selectedPath, setSelectedPath] = useState("");
  const switchWorkspace = useSwitchWorkspace(closeWorkspaceQuickSwitch);

  useEffect(() => {
    if (!quickSwitch.open) return;
    setSelectedPath(quickSwitch.paths[0] ?? "");
  }, [quickSwitch.open, quickSwitch.paths]);

  const name = workspaceName(selectedPath, "memorph");

  return (
    <Dialog
      open={quickSwitch.open}
      onOpenChange={(open) => !open && closeWorkspaceQuickSwitch()}
    >
      <DialogContent className="sm:max-w-md" data-workspace-quick-switch-dialog>
        <DialogHeader>
          <DialogTitle>切换工作空间</DialogTitle>
          <DialogDescription>
            确认后加载该工作空间下的会话与 Skill 数据。
          </DialogDescription>
        </DialogHeader>

        {quickSwitch.paths.length > 1 ? (
          <div className="grid gap-2">
            <p className="text-muted-foreground text-sm">
              多个路径对应同名工作空间，请选择一个：
            </p>
            {quickSwitch.paths.map((path) => (
              <button
                key={path}
                type="button"
                className={`rounded-md border p-3 text-left transition-colors ${
                  selectedPath === path
                    ? "border-primary bg-primary/5"
                    : "hover:bg-muted"
                }`}
                onClick={() => setSelectedPath(path)}
              >
                <strong className="block truncate">{workspaceName(path, "memorph")}</strong>
                <PathText value={path} wrap="all" className="mt-1 text-xs" />
              </button>
            ))}
          </div>
        ) : (
          <div className="rounded-md border p-3">
            <strong className="block text-base">{name}</strong>
            <PathText value={selectedPath} wrap="all" className="mt-2 text-sm" />
          </div>
        )}

        <DialogFooter className="gap-2 sm:justify-end">
          <Button
            type="button"
            variant="outline"
            onClick={closeWorkspaceQuickSwitch}
            disabled={switchWorkspace.isPending}
          >
            取消
          </Button>
          <Button
            type="button"
            disabled={!selectedPath.trim() || switchWorkspace.isPending}
            onClick={() => switchWorkspace.mutate(selectedPath.trim())}
          >
            {switchWorkspace.isPending ? <Spinner /> : null}
            切换
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
