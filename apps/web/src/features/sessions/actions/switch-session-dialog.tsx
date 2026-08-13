import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Controller, useForm, useWatch } from "react-hook-form";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { TargetAgentPicker } from "@/features/sessions/actions/target-agent-picker";
import { defaultSwitchTarget, switchSchema } from "@/features/sessions/model/schemas";
import type { SwitchForm } from "@/features/sessions/model/schemas";
import { WorkspacePathPicker } from "@/features/workspaces/workspace-path-picker";
import { nativeForkSession, switchSession } from "@/lib/api";
import { useI18n } from "@/lib/i18n-context";
import { queryKeys } from "@/lib/query-keys";
import type { MetaPayload, ProviderInfo } from "@/lib/types";

export function SwitchSessionDialog({
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
  const queryClient = useQueryClient();
  const { t } = useI18n();
  const form = useForm<SwitchForm>({
    resolver: zodResolver(switchSchema),
    defaultValues: { to: "", target_title: "", to_dir: "" },
  });

  useEffect(() => {
    if (!open || !target) return;
    form.reset({
      to: defaultSwitchTarget(providers, target.providerId),
      target_title: target.title || "",
      to_dir: target.workspace || meta?.selected_workspace || "",
    });
  }, [form, meta?.selected_workspace, open, providers, target]);

  const switchMutation = useMutation({
    mutationFn: ({ values, moveOriginal }: { values: SwitchForm; moveOriginal: boolean }) => {
      if (!target) throw new Error("Missing session target");
      return switchSession({
        from: target.providerId,
        to: values.to,
        session_id: target.sessionId,
        to_dir: values.to_dir?.trim() || null,
        target_title: values.target_title?.trim() || null,
        move_original: moveOriginal,
      });
    },
    onSuccess: async (result, variables) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      toast.success(result.removed_original ? t("switchMoved") : t("switchCopied"), {
        description: `${result.from_name} -> ${result.to_name}: ${result.target_session_id}`,
      });
      navigate(`/sessions/${encodeURIComponent(variables.values.to)}/${encodeURIComponent(result.target_session_id)}`);
    },
  });

  const nativeForkMutation = useMutation({
    mutationFn: () => {
      if (!target) throw new Error("Missing session target");
      return nativeForkSession({ provider: target.providerId, session_id: target.sessionId });
    },
    onSuccess: async (result) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
      ]);
      onOpenChange(false);
      toast.success(t("sessionNativeForkCreated"), { description: `${result.to_name}: ${result.target_session_id}` });
      navigate(`/sessions/${encodeURIComponent(result.to_name)}/${encodeURIComponent(result.target_session_id)}`);
    },
  });

  function submitSwitch(values: SwitchForm, moveOriginal: boolean) {
    switchMutation.mutate({ values, moveOriginal });
  }

  const exportProviders = providers.filter((provider) => provider.export);
  const selectedTarget = useWatch({ control: form.control, name: "to" });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex! h-[min(34rem,calc(100dvh-2rem))] w-full flex-col gap-4 overflow-hidden sm:max-w-xl"
        data-switch-session-dialog
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("switch")}</DialogTitle>
        </DialogHeader>
        <DialogForm className="flex min-h-0 min-w-0 flex-1 flex-col" onSubmit={form.handleSubmit((values) => submitSwitch(values, false))}>
          <input type="hidden" name="from" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup className="flex min-h-0 flex-1 flex-col gap-4" data-switch-modal-grid>
            <Field className="flex min-h-0 flex-1 flex-col" data-invalid={Boolean(form.formState.errors.to)}>
              <FieldLabel>{t("sessionTargetAgent")}</FieldLabel>
              <Controller
                control={form.control}
                name="to"
                render={({ field }) => (
                  <TargetAgentPicker
                    providers={exportProviders}
                    value={field.value}
                    onChange={(providerId) => field.onChange(providerId)}
                  />
                )}
              />
              {form.formState.errors.to ? <FieldDescription>{form.formState.errors.to.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.target_title)}>
              <FieldLabel htmlFor="switch-copy-title">{t("sessionCopyTitle")}</FieldLabel>
              <Input id="switch-copy-title" placeholder={target?.title || target?.sessionId || t("sessionTitlePlaceholder")} {...form.register("target_title")} />
              {form.formState.errors.target_title ? <FieldDescription>{form.formState.errors.target_title.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="switch-target-dir">{t("sessionTargetDirectory")}</FieldLabel>
              <Controller
                control={form.control}
                name="to_dir"
                render={({ field }) => (
                  <WorkspacePathPicker
                    id="switch-target-dir"
                    value={field.value ?? ""}
                    onChange={field.onChange}
                    placeholder={t("sessionWorkspacePath")}
                  />
                )}
              />
            </Field>
          </FieldGroup>

          <DialogFormFooter
            className="shrink-0"
            onCancel={() => onOpenChange(false)}
            cancelLabel={t("cancel")}
            submitDisabled={!target || !exportProviders.length}
            submitLabel={t("runSwitch")}
            submitting={switchMutation.isPending}
          >
            <Button
              type="button"
              variant="outline"
              disabled={
                !target ||
                selectedTarget !== target.providerId ||
                !providers.find((provider) => provider.id === target.providerId)?.native_fork ||
                switchMutation.isPending ||
                nativeForkMutation.isPending
              }
              title={providers.find((provider) => provider.id === target?.providerId)?.native_fork ? undefined : t("sessionNativeForkUnavailable")}
              onClick={() => nativeForkMutation.mutate()}
            >
              {t("sessionNativeFork")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={!target || !exportProviders.length || switchMutation.isPending || nativeForkMutation.isPending}
              onClick={form.handleSubmit((values) => submitSwitch(values, true))}
            >
              {t("sessionMove")}
            </Button>
          </DialogFormFooter>
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
