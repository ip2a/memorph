import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { listSessions } from "@/lib/api";
import { workspaceName } from "@/components/shared/workspace-name";
import { useUiStore } from "@/stores/ui-store";

export function useSwitchWorkspace(onDone?: () => void) {
  const queryClient = useQueryClient();
  const setSelectedWorkspace = useUiStore((state) => state.setSelectedWorkspace);

  return useMutation({
    mutationFn: async (workspace: string) => {
      await listSessions({ all: true, fields: "minimal", limit: 1, workspace });
      return workspace;
    },
    onSuccess: (workspace) => {
      setSelectedWorkspace(workspace);
      onDone?.();
      toast.success("Workspace switched", {
        description: workspaceName(workspace, "memorph"),
      });
      void queryClient.invalidateQueries();
    },
  });
}
