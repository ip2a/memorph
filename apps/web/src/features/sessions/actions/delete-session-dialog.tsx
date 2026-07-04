import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Spinner } from "@/components/ui/spinner";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { deleteSession } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function DeleteSessionDialog({
  target,
  open,
  onOpenChange,
  returnHomeOnSuccess = false,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnHomeOnSuccess?: boolean;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const deleteMutation = useMutation({
    mutationFn: () => {
      if (!target) throw new Error("Missing session target");
      return deleteSession(target.providerId, target.sessionId);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      toast.success("Deleted", { description: target ? `${target.providerId}: ${target.sessionId}` : undefined });
      if (returnHomeOnSuccess) navigate("/");
    },
  });

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent data-delete-session-dialog>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove</AlertDialogTitle>
          <AlertDialogDescription>
            Delete this session from its provider. This matches the legacy remove workflow and cannot be undone here.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="grid gap-1 rounded-md border p-3 font-mono text-xs" data-delete-session-target>
          <span>{target?.providerId || "-"}</span>
          <span className="break-all text-muted-foreground">{target?.sessionId || "-"}</span>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deleteMutation.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={!target || deleteMutation.isPending}
            onClick={(event) => {
              event.preventDefault();
              deleteMutation.mutate();
            }}
          >
            {deleteMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
