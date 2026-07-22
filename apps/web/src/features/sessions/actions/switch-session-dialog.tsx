import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FolderOpenIcon } from "lucide-react";
import { useEffect } from "react";
import { useForm, useWatch } from "react-hook-form";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { defaultSwitchTarget, switchSchema, workspaceOptions } from "@/features/sessions/model/schemas";
import type { SwitchForm } from "@/features/sessions/model/schemas";
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
      toast.success("Native fork created", { description: `${result.to_name}: ${result.target_session_id}` });
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
      <DialogContent className="sm:max-w-2xl" data-switch-session-dialog>
        <DialogHeader>
          <DialogTitle>{t("switch")}</DialogTitle>
          <DialogDescription>{t("switchDialogDescription")}</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => submitSwitch(values, false))}>
          <input type="hidden" name="from" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup className="grid gap-4 sm:grid-cols-[minmax(0,0.75fr)_minmax(0,1.25fr)]" data-switch-modal-grid>
            <Field data-invalid={Boolean(form.formState.errors.to)}>
              <FieldLabel htmlFor="switch-target-provider">Target Provider</FieldLabel>
              <Select value={selectedTarget} onValueChange={(value) => form.setValue("to", value, { shouldValidate: true })}>
                <SelectTrigger id="switch-target-provider" className="w-full" aria-invalid={Boolean(form.formState.errors.to)}>
                  <SelectValue placeholder="Provider" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {exportProviders.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>
                        {provider.name}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {form.formState.errors.to ? <FieldDescription>{form.formState.errors.to.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.target_title)}>
              <FieldLabel htmlFor="switch-copy-title">Copy Session Title</FieldLabel>
              <Input id="switch-copy-title" placeholder={target?.title || target?.sessionId || "Session title"} {...form.register("target_title")} />
              {form.formState.errors.target_title ? <FieldDescription>{form.formState.errors.target_title.message}</FieldDescription> : null}
            </Field>

            <Field className="sm:col-span-2">
              <FieldLabel htmlFor="switch-target-dir">Target Dir</FieldLabel>
              <InputGroup>
                <InputGroupInput id="switch-target-dir" list="known-workspaces" placeholder="Workspace path" {...form.register("to_dir")} />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" disabled>
                    <FolderOpenIcon data-icon="inline-start" />
                    Browse
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>Copy Session Title is used as the target session display title when provided.</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceOptions(meta).map((item) => (
              <option key={item.path} value={item.path} />
            ))}
          </datalist>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
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
              title={providers.find((provider) => provider.id === target?.providerId)?.native_fork ? undefined : "This provider does not expose a verified native fork API"}
              onClick={() => nativeForkMutation.mutate()}
            >
              Native Fork
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={!target || !exportProviders.length || switchMutation.isPending || nativeForkMutation.isPending}
              onClick={form.handleSubmit((values) => submitSwitch(values, true))}
            >
              Move
            </Button>
          </DialogFormFooter>
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
