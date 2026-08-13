import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Controller, useForm, useWatch } from "react-hook-form";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldLegend, FieldSet, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { createSyncSchema, defaultSwitchTarget, syncTargetProviders } from "@/features/sessions/model/schemas";
import type { CreateSyncForm } from "@/features/sessions/model/schemas";
import { WorkspacePathPicker } from "@/features/workspaces/workspace-path-picker";
import { createSyncGroup } from "@/lib/api";
import { useI18n } from "@/lib/i18n-context";
import { queryKeys } from "@/lib/query-keys";
import type { MetaPayload, ProviderInfo } from "@/lib/types";

export function CreateSyncDialog({
  target,
  open,
  onOpenChange,
  providers,
  meta,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  providers: ProviderInfo[];
  meta?: MetaPayload;
}) {
  const navigate = useNavigate();
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const form = useForm<CreateSyncForm>({
    resolver: zodResolver(createSyncSchema),
    defaultValues: { title: "", to_dir: "", targets: [] },
  });

  const candidateTargets = syncTargetProviders(providers, target?.providerId || "");
  const selectedTargets = useWatch({ control: form.control, name: "targets" }) ?? [];

  useEffect(() => {
    if (!open || !target) return;
    const nextTargets = syncTargetProviders(providers, target.providerId);
    const preferred = defaultSwitchTarget(providers, target.providerId);
    const defaultTarget = nextTargets.some((provider) => provider.id === preferred)
      ? preferred
      : nextTargets[0]?.id || "";
    form.reset({
      title: target.title || "",
      to_dir: target.workspace || meta?.selected_workspace || "",
      targets: defaultTarget ? [defaultTarget] : [],
    });
  }, [form, meta?.selected_workspace, open, providers, target]);

  const createSyncMutation = useMutation({
    mutationFn: (values: CreateSyncForm) => {
      if (!target) throw new Error("Missing session target");
      return createSyncGroup({
        provider: target.providerId,
        session_id: target.sessionId,
        targets: values.targets.filter((providerId) => providerId !== target.providerId),
        to_dir: values.to_dir?.trim() || null,
        title: values.title?.trim() || null,
      });
    },
    onSuccess: async (group) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      toast.success(t("sessionSyncCreated"), {
        description: `${group.id} · holdings=${group.holdings.length}`,
      });
      navigate(`/sync?group=${encodeURIComponent(group.id)}`);
    },
  });

  function toggleTarget(providerId: string, checked: boolean) {
    const current = new Set(form.getValues("targets"));
    if (checked) {
      current.add(providerId);
    } else {
      current.delete(providerId);
    }
    form.setValue("targets", [...current], { shouldDirty: true, shouldValidate: true });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-create-sync-dialog>
        <DialogHeader>
          <DialogTitle>{t("sessionCreateSync")}</DialogTitle>
          <DialogDescription>{t("sessionCreateSyncDescription")}</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => createSyncMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup data-create-sync-modal-stack>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="sync-title">{t("sessionTitleLabel")}</FieldLabel>
              <Input id="sync-title" placeholder={target?.title || target?.sessionId || t("sessionCreateSync")} {...form.register("title")} />
              {form.formState.errors.title ? <FieldDescription>{form.formState.errors.title.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="sync-target-dir">{t("sessionTargetDirectory")}</FieldLabel>
              <Controller
                control={form.control}
                name="to_dir"
                render={({ field }) => (
                  <WorkspacePathPicker
                    id="sync-target-dir"
                    value={field.value ?? ""}
                    onChange={field.onChange}
                    placeholder={t("sessionWorkspacePath")}
                  />
                )}
              />
              <FieldDescription>{t("sessionTargetDirectoryDescription")}</FieldDescription>
            </Field>

            <FieldSet data-invalid={Boolean(form.formState.errors.targets)} data-create-sync-target-providers>
              <FieldLegend>{t("sessionTargetProviders")}</FieldLegend>
              <FieldGroup data-slot="checkbox-group">
                {candidateTargets.length ? (
                  candidateTargets.map((provider) => {
                    const checked = selectedTargets.includes(provider.id);
                    return (
                      <Field key={provider.id} orientation="horizontal">
                        <Checkbox
                          id={`sync-target-${provider.id}`}
                          name="targets"
                          value={provider.id}
                          checked={checked}
                          onCheckedChange={(value) => toggleTarget(provider.id, value === true)}
                          aria-invalid={Boolean(form.formState.errors.targets)}
                        />
                        <FieldContent>
                          <FieldLabel htmlFor={`sync-target-${provider.id}`}>
                            <FieldTitle>{provider.name}</FieldTitle>
                          </FieldLabel>
                          <FieldDescription>{provider.id}</FieldDescription>
                        </FieldContent>
                      </Field>
                    );
                  })
                ) : (
                  <FieldDescription>{t("sessionNoSyncTargets")}</FieldDescription>
                )}
              </FieldGroup>
              {form.formState.errors.targets ? <FieldDescription>{form.formState.errors.targets.message}</FieldDescription> : null}
            </FieldSet>
          </FieldGroup>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            cancelLabel={t("cancel")}
            submitDisabled={!target || !candidateTargets.length}
            submitLabel={t("sessionCreate")}
            submitting={createSyncMutation.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
