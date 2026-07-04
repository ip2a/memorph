import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { renameSchema } from "@/features/sessions/model/schemas";
import type { RenameForm } from "@/features/sessions/model/schemas";
import { renameSession } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function RenameSessionDialog({
  target,
  open,
  onOpenChange,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const form = useForm<RenameForm>({
    resolver: zodResolver(renameSchema),
    values: { title: target?.title || "" },
  });

  const renameMutation = useMutation({
    mutationFn: (values: RenameForm) => {
      if (!target) throw new Error("Missing session target");
      return renameSession(target.providerId, target.sessionId, { title: values.title.trim() });
    },
    onSuccess: async (result) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        target ? queryClient.invalidateQueries({ queryKey: queryKeys.session(target.providerId, target.sessionId) }) : Promise.resolve(),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      toast.success("Rename", { description: result.warning || result.display_title });
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-rename-session-dialog>
        <DialogHeader>
          <DialogTitle>Rename</DialogTitle>
          <DialogDescription>Update the display title for this session.</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => renameMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="rename-session-title">Title</FieldLabel>
              <Input id="rename-session-title" aria-invalid={Boolean(form.formState.errors.title)} {...form.register("title")} />
              {form.formState.errors.title ? <FieldDescription>{form.formState.errors.title.message}</FieldDescription> : null}
            </Field>
          </FieldGroup>
          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            submitDisabled={!target}
            submitLabel="Save"
            submitting={renameMutation.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
