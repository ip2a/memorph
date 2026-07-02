import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FolderOpenIcon } from "lucide-react";
import { useEffect } from "react";
import { useForm, useWatch } from "react-hook-form";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { z } from "zod";
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
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldLegend, FieldSet, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { createSyncGroup, deleteSession, exportSession, renameSession, switchSession } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { MetaPayload, ProviderInfo } from "@/lib/types";

const renameSchema = z.object({
  title: z.string().trim().min(1, "Enter a title."),
});

type RenameForm = z.infer<typeof renameSchema>;

const switchSchema = z.object({
  to: z.string().min(1, "Choose a target provider."),
  target_title: z.string().trim().optional(),
  to_dir: z.string().trim().optional(),
});

type SwitchForm = z.infer<typeof switchSchema>;

const exportSchema = z.object({
  output_prefix: z.string().trim().min(1, "Enter an output file name."),
  format: z.string().min(1, "Choose a format."),
  output_dir: z.string().trim().optional(),
});

type ExportForm = z.infer<typeof exportSchema>;

const createSyncSchema = z.object({
  title: z.string().trim().optional(),
  to_dir: z.string().trim().optional(),
  targets: z.array(z.string()).min(1, "Choose at least one target provider."),
});

type CreateSyncForm = z.infer<typeof createSyncSchema>;

function defaultSwitchTarget(providers: ProviderInfo[], sourceProviderId: string) {
  const candidates = providers.filter((provider) => provider.id !== sourceProviderId && provider.export);
  if (!candidates.length) return "";
  if (sourceProviderId === "codex") {
    return candidates.find((provider) => provider.id === "claude")?.id ?? candidates[0].id;
  }
  return candidates[0].id;
}

function workspaceOptions(meta?: MetaPayload) {
  return meta?.workspaces ?? [];
}

function providerLabel(providers: ProviderInfo[], providerId: string) {
  return providers.find((provider) => provider.id === providerId)?.name ?? providerId;
}

function syncTargetProviders(providers: ProviderInfo[], sourceProviderId: string) {
  return providers.filter((provider) => provider.id !== sourceProviderId && provider.export);
}

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
      toast.success(result.removed_original ? "Moved" : "Copied", {
        description: `${result.from_name} -> ${result.to_name}: ${result.target_session_id}`,
      });
      navigate(`/sessions/${encodeURIComponent(variables.values.to)}/${encodeURIComponent(result.target_session_id)}`);
    },
  });

  function submitSwitch(values: SwitchForm, moveOriginal: boolean) {
    switchMutation.mutate({ values, moveOriginal });
  }

  const exportProviders = providers.filter((provider) => provider.id !== target?.providerId && provider.export);
  const selectedTarget = useWatch({ control: form.control, name: "to" });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-switch-session-dialog>
        <DialogHeader>
          <DialogTitle>Copy</DialogTitle>
          <DialogDescription>Copy this session to another provider while keeping the legacy switch workflow placement.</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-5" onSubmit={form.handleSubmit((values) => submitSwitch(values, false))}>
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

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!target || !exportProviders.length || switchMutation.isPending}>
              {switchMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              Copy
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={!target || !exportProviders.length || switchMutation.isPending}
              onClick={form.handleSubmit((values) => submitSwitch(values, true))}
            >
              Move
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function ExportSessionDialog({
  target,
  open,
  onOpenChange,
  meta,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meta?: MetaPayload;
}) {
  const form = useForm<ExportForm>({
    resolver: zodResolver(exportSchema),
    defaultValues: { output_prefix: "", format: "both", output_dir: "" },
  });

  useEffect(() => {
    if (!open || !target) return;
    form.reset({
      output_prefix: target.sessionId,
      format: "both",
      output_dir: target.workspace || meta?.selected_workspace || "",
    });
  }, [form, meta?.selected_workspace, open, target]);

  const exportMutation = useMutation({
    mutationFn: (values: ExportForm) => {
      if (!target) throw new Error("Missing session target");
      return exportSession({
        provider: target.providerId,
        session_id: target.sessionId,
        output_prefix: values.output_prefix.trim() || null,
        format: values.format,
        output_dir: values.output_dir?.trim() || null,
      });
    },
    onSuccess: (result) => {
      onOpenChange(false);
      toast.success("Exported", {
        description: result.files.length ? result.files.join("\n") : providerLabel([], target?.providerId || ""),
      });
    },
  });

  const selectedFormat = useWatch({ control: form.control, name: "format" });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-export-session-dialog>
        <DialogHeader>
          <DialogTitle>Export</DialogTitle>
          <DialogDescription>Write this session to export files using the legacy export workflow fields.</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-5" onSubmit={form.handleSubmit((values) => exportMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_11rem]" data-export-modal-grid>
            <Field data-invalid={Boolean(form.formState.errors.output_prefix)} className="sm:col-span-2">
              <FieldLabel htmlFor="export-output-prefix">Output File Name</FieldLabel>
              <Input
                id="export-output-prefix"
                placeholder="Output file name"
                aria-invalid={Boolean(form.formState.errors.output_prefix)}
                {...form.register("output_prefix")}
              />
              {form.formState.errors.output_prefix ? <FieldDescription>{form.formState.errors.output_prefix.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.format)}>
              <FieldLabel htmlFor="export-format">Format</FieldLabel>
              <Select value={selectedFormat} onValueChange={(value) => form.setValue("format", value, { shouldValidate: true })}>
                <SelectTrigger id="export-format" className="w-full" aria-invalid={Boolean(form.formState.errors.format)}>
                  <SelectValue placeholder="Format" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {[
                      "json",
                      "md",
                      "html",
                      "morph",
                      "both",
                    ].map((format) => (
                      <SelectItem key={format} value={format}>
                        {format}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {form.formState.errors.format ? <FieldDescription>{form.formState.errors.format.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="export-output-dir">Export Directory</FieldLabel>
              <InputGroup>
                <InputGroupInput id="export-output-dir" list="known-workspaces" placeholder="Export directory" {...form.register("output_dir")} />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" disabled>
                    <FolderOpenIcon data-icon="inline-start" />
                    Browse
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>Defaults to the current workspace when left unchanged.</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceOptions(meta).map((item) => (
              <option key={item.path} value={item.path} />
            ))}
          </datalist>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!target || exportMutation.isPending}>
              {exportMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              Export
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

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
      toast.success("Sync created", {
        description: `${group.id} · holdings=${group.holdings.length}`,
      });
      navigate(`/sync/${encodeURIComponent(group.id)}`);
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
          <DialogTitle>Create Sync</DialogTitle>
          <DialogDescription>Create a sync group from this session using the legacy field order.</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-5" onSubmit={form.handleSubmit((values) => createSyncMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup data-create-sync-modal-stack>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="sync-title">Title</FieldLabel>
              <Input id="sync-title" placeholder={target?.title || target?.sessionId || "Sync title"} {...form.register("title")} />
              {form.formState.errors.title ? <FieldDescription>{form.formState.errors.title.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="sync-target-dir">Target Dir</FieldLabel>
              <InputGroup>
                <InputGroupInput id="sync-target-dir" list="known-workspaces" placeholder="Workspace path" {...form.register("to_dir")} />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" disabled>
                    <FolderOpenIcon data-icon="inline-start" />
                    Browse
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>Use the current workspace unless another target directory is selected.</FieldDescription>
            </Field>

            <FieldSet data-invalid={Boolean(form.formState.errors.targets)} data-create-sync-target-providers>
              <FieldLegend>Target Providers</FieldLegend>
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
                  <FieldDescription>No sync targets</FieldDescription>
                )}
              </FieldGroup>
              {form.formState.errors.targets ? <FieldDescription>{form.formState.errors.targets.message}</FieldDescription> : null}
            </FieldSet>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceOptions(meta).map((item) => (
              <option key={item.path} value={item.path} />
            ))}
          </datalist>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!target || !candidateTargets.length || createSyncMutation.isPending}>
              {createSyncMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              Create
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

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
        <form className="flex flex-col gap-5" onSubmit={form.handleSubmit((values) => renameMutation.mutate(values))}>
          <input type="hidden" name="provider" value={target?.providerId || ""} />
          <input type="hidden" name="session_id" value={target?.sessionId || ""} />
          <FieldGroup>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="rename-session-title">Title</FieldLabel>
              <Input id="rename-session-title" aria-invalid={Boolean(form.formState.errors.title)} {...form.register("title")} />
              {form.formState.errors.title ? <FieldDescription>{form.formState.errors.title.message}</FieldDescription> : null}
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!target || renameMutation.isPending}>
              {renameMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

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
