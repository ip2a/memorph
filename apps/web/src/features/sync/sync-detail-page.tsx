import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeftIcon, ArrowRightIcon, FolderOpenIcon, GitBranchIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useForm, useWatch } from "react-hook-form";
import { Link, useNavigate, useParams } from "react-router-dom";
import { toast } from "sonner";
import { z } from "zod";
import { DetailHeader } from "@/components/shared/detail-header";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { EntityRow } from "@/components/shared/entity-row";
import { MetaLine } from "@/components/shared/meta-line";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { bindSyncGroup, getMeta, listProviders, removeSyncGroup, renameSyncGroup, runSyncGroup, unbindSyncHolding } from "@/lib/api";
import { formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type { MetaPayload, ProviderInfo, SyncGroup, SyncHolding, SyncReport } from "@/lib/types";
import { useSyncGroup } from "@/features/sync/queries";
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
import { Badge } from "@/components/ui/badge";
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
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";

const bindSchema = z.object({
  provider: z.string().min(1, "Choose a provider."),
  session_id: z.string().trim().optional(),
  to_dir: z.string().trim().optional(),
});

type BindForm = z.infer<typeof bindSchema>;

const renameSchema = z.object({
  title: z.string().trim().min(1, "Enter a title."),
});

type RenameForm = z.infer<typeof renameSchema>;

function providerLabel(providers: ProviderInfo[], providerId?: string | null) {
  if (!providerId) return "-";
  return providers.find((provider) => provider.id === providerId)?.name ?? providerId;
}

function defaultBindProvider(providers: ProviderInfo[], group: SyncGroup | null) {
  const existing = new Set(group?.holdings.map((holding) => holding.provider) ?? []);
  const writable = providers.filter((provider) => provider.export && !existing.has(provider.id));
  return writable[0]?.id ?? providers[0]?.id ?? "";
}

function workspaceOptions(meta?: MetaPayload) {
  return meta?.workspaces ?? [];
}

function syncReportDescription(report: SyncReport) {
  return `${report.source_provider} · success=${report.success.length} · errors=${report.errors.length}`;
}

function firstReportLine(report: SyncReport) {
  return report.errors[0] ?? report.success[0] ?? report.source_holding_id;
}

export function SyncDetailPage() {
  const { groupId = "" } = useParams();
  const queryClient = useQueryClient();
  const syncGroup = useSyncGroup(groupId);
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: listProviders });
  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });
  const [bindOpen, setBindOpen] = useState(false);
  const [syncSource, setSyncSource] = useState<SyncHolding | null>(null);
  const [unbindTarget, setUnbindTarget] = useState<SyncHolding | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);

  const group = syncGroup.data;
  const providerItems = useMemo(() => providers.data ?? [], [providers.data]);

  const runLatestMutation = useMutation({
    mutationFn: () => runSyncGroup({ group_id: groupId }),
    onSuccess: async (report) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(groupId) }),
      ]);
      toast.success("Start Execution", { description: syncReportDescription(report) });
    },
  });

  if (syncGroup.isLoading) return <PageSkeleton />;
  if (syncGroup.error) return <PageError title="Sync group failed to load" message={syncGroup.error.message} />;
  if (!group) return <PageEmpty title="Sync group not found" description="Return to the sync list and choose another group." />;

  return (
    <>
      <TwoPanePage data-sync-detail-layout>
        <PanelCard className="min-h-0" data-sync-detail-actions>
          <section className="flex flex-col gap-3 border-b pb-4">
            <div className="flex items-center gap-2">
              <GitBranchIcon aria-hidden />
              <h2 className="truncate text-lg font-semibold">{group.title || group.id}</h2>
            </div>
            <p className="break-all font-mono text-xs text-muted-foreground">{group.id}</p>
          </section>

          <ScrollArea className="min-h-0 flex-1 pr-3">
            <div className="flex flex-col gap-4">
              <div className="flex flex-col gap-2 text-sm">
                <MetaLine label="Provider" value={providerLabel(providerItems, group.source_provider)} />
                <MetaLine label="Sync Title" value={group.title} />
                <MetaLine label="Holdings" value={String(group.holdings.length)} />
                <MetaLine label="Created At" value={formatDateTime(group.created_at)} />
                <MetaLine label="Updated At" value={formatDateTime(group.updated_at)} />
              </div>

              <Separator />

              <div className="flex flex-col gap-2" data-sync-detail-row-actions>
                <Button variant="default" onClick={() => runLatestMutation.mutate()} disabled={runLatestMutation.isPending}>
                  {runLatestMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
                  Start Execution
                </Button>
                <Button variant="outline" onClick={() => setBindOpen(true)}>Add Holding</Button>
                <Button variant="outline" onClick={() => setRenameOpen(true)}>Rename</Button>
                <Button variant="destructive" onClick={() => setRemoveOpen(true)}>Remove</Button>
                <Button asChild variant="outline">
                  <Link to="/sync">
                    <ArrowLeftIcon data-icon="inline-start" />
                    Back
                  </Link>
                </Button>
              </div>
            </div>
          </ScrollArea>
        </PanelCard>

        <PanelCard variant="plain" className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-4" data-sync-holdings-panel>
          <DetailHeader
            title={group.title}
            badges={<Badge variant="secondary">Holdings</Badge>}
            description={`${group.holdings.length} linked sessions`}
          />

          <ScrollArea className="min-h-0 pr-3">
            {group.holdings.length === 0 ? (
              <PageEmpty title="No holdings" description="Add a holding from the status panel to bind this sync group." />
            ) : (
              <div className="flex flex-col gap-2" data-sync-holding-grid>
                {group.holdings.map((holding) => (
                  <HoldingCard
                    key={holding.id}
                    group={group}
                    holding={holding}
                    providerName={providerLabel(providerItems, holding.provider)}
                    onSyncFrom={() => setSyncSource(holding)}
                    onUnbind={() => setUnbindTarget(holding)}
                  />
                ))}
              </div>
            )}
          </ScrollArea>
        </PanelCard>
      </TwoPanePage>

      <BindSyncHoldingDialog
        group={group}
        open={bindOpen}
        onOpenChange={setBindOpen}
        providers={providerItems}
        meta={meta.data}
      />
      <SyncFromHoldingDialog group={group} holding={syncSource} open={Boolean(syncSource)} onOpenChange={(open) => !open && setSyncSource(null)} />
      <UnbindHoldingDialog group={group} holding={unbindTarget} open={Boolean(unbindTarget)} onOpenChange={(open) => !open && setUnbindTarget(null)} />
      <RenameSyncGroupDialog group={group} open={renameOpen} onOpenChange={setRenameOpen} />
      <RemoveSyncGroupDialog group={group} open={removeOpen} onOpenChange={setRemoveOpen} />
    </>
  );
}

function HoldingCard({
  group,
  holding,
  providerName,
  onSyncFrom,
  onUnbind,
}: {
  group: SyncGroup;
  holding: SyncHolding;
  providerName: string;
  onSyncFrom: () => void;
  onUnbind: () => void;
}) {
  const sessionHref = `/sessions/${encodeURIComponent(holding.provider)}/${encodeURIComponent(holding.session_id)}`;
  return (
    <EntityRow
      data-sync-holding-card
      actionsProps={{ "data-sync-holding-actions": true }}
      actions={(
        <>
          <Button asChild variant="outline">
            <Link to={sessionHref}>
              Open Session
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          </Button>
          <Button variant="outline" onClick={onSyncFrom} data-group-id={group.id} data-holding-id={holding.id}>
            Sync From This
          </Button>
          <Button variant="destructive" onClick={onUnbind} data-group-id={group.id} data-holding-id={holding.id}>
            Unbind
          </Button>
        </>
      )}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <Link to={sessionHref} className="truncate text-sm font-medium hover:underline">
          {providerName}
        </Link>
        <p className="truncate font-mono text-xs text-muted-foreground">{holding.session_id}</p>
        <div className="flex flex-col gap-1 text-sm">
          <MetaLine label="Workspace" value={<PathText value={holding.target_dir} wrap="all" />} />
          <MetaLine label="Last Active At" value={formatDateTime(holding.last_active_at)} />
          <MetaLine label="Last Sync" value={formatDateTime(holding.last_sync_at)} />
          <MetaLine label="Sync From" value={holding.last_sync_from || "-"} />
          <MetaLine label="Error" value={holding.last_error || "-"} destructive={Boolean(holding.last_error)} />
        </div>
      </div>
    </EntityRow>
  );
}

function BindSyncHoldingDialog({
  group,
  open,
  onOpenChange,
  providers,
  meta,
}: {
  group: SyncGroup;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  providers: ProviderInfo[];
  meta?: MetaPayload;
}) {
  const queryClient = useQueryClient();
  const form = useForm<BindForm>({
    resolver: zodResolver(bindSchema),
    defaultValues: { provider: "", session_id: "", to_dir: "" },
  });
  const selectedProvider = useWatch({ control: form.control, name: "provider" });

  useEffect(() => {
    if (!open) return;
    form.reset({
      provider: defaultBindProvider(providers, group),
      session_id: "",
      to_dir: meta?.selected_workspace || "",
    });
  }, [form, group, meta?.selected_workspace, open, providers]);

  const bindMutation = useMutation({
    mutationFn: (values: BindForm) => bindSyncGroup({
      group_id: group.id,
      provider: values.provider,
      session_id: values.session_id?.trim() || null,
      to_dir: values.to_dir?.trim() || null,
    }),
    onSuccess: async (holding) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(group.id) }),
      ]);
      toast.success("Add Holding", { description: `${holding.provider}: ${holding.session_id}` });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-bind-sync-holding-dialog>
        <DialogHeader>
          <DialogTitle>Add Holding</DialogTitle>
          <DialogDescription>Bind another provider session to this sync group.</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => bindMutation.mutate(values))}>
          <input type="hidden" name="group_id" value={group.id} />
          <FieldGroup data-bind-sync-modal-stack>
            <Field data-invalid={Boolean(form.formState.errors.provider)}>
              <FieldLabel htmlFor="bind-provider">Provider</FieldLabel>
              <Select value={selectedProvider} onValueChange={(value) => form.setValue("provider", value, { shouldValidate: true })}>
                <SelectTrigger id="bind-provider" className="w-full" aria-invalid={Boolean(form.formState.errors.provider)}>
                  <SelectValue placeholder="Provider" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {providers.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>{provider.name}</SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {form.formState.errors.provider ? <FieldDescription>{form.formState.errors.provider.message}</FieldDescription> : null}
            </Field>

            <Field data-invalid={Boolean(form.formState.errors.session_id)}>
              <FieldLabel htmlFor="bind-session-id">Session Id</FieldLabel>
              <Input id="bind-session-id" placeholder="Empty creates new holding" {...form.register("session_id")} />
              {form.formState.errors.session_id ? <FieldDescription>{form.formState.errors.session_id.message}</FieldDescription> : null}
            </Field>

            <Field>
              <FieldLabel htmlFor="bind-target-dir">Target Dir</FieldLabel>
              <InputGroup>
                <InputGroupInput id="bind-target-dir" list="known-workspaces" placeholder="Workspace path" {...form.register("to_dir")} />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" disabled>
                    <FolderOpenIcon data-icon="inline-start" />
                    Browse
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>Leave Session Id empty to create a new provider holding when supported.</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceOptions(meta).map((item) => <option key={item.path} value={item.path} />)}
          </datalist>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            submitDisabled={!providers.length}
            submitLabel="Add Holding"
            submitting={bindMutation.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}

function SyncFromHoldingDialog({
  group,
  holding,
  open,
  onOpenChange,
}: {
  group: SyncGroup;
  holding: SyncHolding | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const syncMutation = useMutation({
    mutationFn: () => {
      if (!holding) throw new Error("Missing sync holding target");
      return runSyncGroup({ group_id: group.id, source_holding_id: holding.id });
    },
    onSuccess: async (report) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(group.id) }),
      ]);
      toast.success("Sync From This", { description: firstReportLine(report) });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-sync-from-holding-dialog>
        <DialogHeader>
          <DialogTitle>Push Sync</DialogTitle>
          <DialogDescription>Use this holding as the source for the sync group.</DialogDescription>
        </DialogHeader>
        <input type="hidden" name="group_id" value={group.id} />
        <input type="hidden" name="holding_id" value={holding?.id || ""} />
        <div className="rounded-md border p-3 text-sm">
          <div className="font-medium">{holding?.provider || "-"}</div>
          <div className="break-all font-mono text-xs text-muted-foreground">{holding?.session_id || "-"}</div>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button type="button" onClick={() => syncMutation.mutate()} disabled={!holding || syncMutation.isPending}>
            {syncMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
            Sync From This
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function UnbindHoldingDialog({
  group,
  holding,
  open,
  onOpenChange,
}: {
  group: SyncGroup;
  holding: SyncHolding | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const unbindMutation = useMutation({
    mutationFn: () => {
      if (!holding) throw new Error("Missing sync holding target");
      return unbindSyncHolding(group.id, holding.id);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(group.id) }),
      ]);
      toast.success("Unbind", { description: holding?.session_id });
      onOpenChange(false);
    },
  });

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent data-unbind-sync-holding-dialog>
        <AlertDialogHeader>
          <AlertDialogTitle>Unbind</AlertDialogTitle>
          <AlertDialogDescription>Remove this holding from the sync group without deleting the provider session.</AlertDialogDescription>
        </AlertDialogHeader>
        <input type="hidden" name="group_id" value={group.id} />
        <input type="hidden" name="holding_id" value={holding?.id || ""} />
        <div className="rounded-md border p-3 text-sm">
          <div className="font-medium">{holding?.provider || "-"}</div>
          <div className="break-all font-mono text-xs text-muted-foreground">{holding?.session_id || "-"}</div>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={unbindMutation.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={!holding || unbindMutation.isPending}
            onClick={(event) => {
              event.preventDefault();
              unbindMutation.mutate();
            }}
          >
            {unbindMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
            Unbind
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function RenameSyncGroupDialog({ group, open, onOpenChange }: { group: SyncGroup; open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const form = useForm<RenameForm>({ resolver: zodResolver(renameSchema), defaultValues: { title: group.title } });

  useEffect(() => {
    if (!open) return;
    form.reset({ title: group.title });
  }, [form, group.title, open]);

  const renameMutation = useMutation({
    mutationFn: (values: RenameForm) => renameSyncGroup(group.id, { title: values.title.trim() }),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(group.id) }),
      ]);
      toast.success("Rename", { description: form.getValues("title") });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-rename-sync-group-dialog>
        <DialogHeader>
          <DialogTitle>Rename</DialogTitle>
          <DialogDescription>Update the sync group title.</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => renameMutation.mutate(values))}>
          <input type="hidden" name="group_id" value={group.id} />
          <FieldGroup>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="rename-sync-detail-title">Title</FieldLabel>
              <Input id="rename-sync-detail-title" aria-invalid={Boolean(form.formState.errors.title)} {...form.register("title")} />
              {form.formState.errors.title ? <FieldDescription>{form.formState.errors.title.message}</FieldDescription> : null}
            </Field>
          </FieldGroup>
          <DialogFormFooter onCancel={() => onOpenChange(false)} submitLabel="Save" submitting={renameMutation.isPending} />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}

function RemoveSyncGroupDialog({ group, open, onOpenChange }: { group: SyncGroup; open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [deleteProviderSessions, setDeleteProviderSessions] = useState(false);

  const removeMutation = useMutation({
    mutationFn: () => removeSyncGroup(group.id, deleteProviderSessions),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups });
      toast.success("Removed", { description: group.title });
      onOpenChange(false);
      navigate("/sync");
    },
  });

  return (
    <AlertDialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setDeleteProviderSessions(false);
        onOpenChange(nextOpen);
      }}
    >
      <AlertDialogContent data-remove-sync-group-dialog>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove</AlertDialogTitle>
          <AlertDialogDescription>Remove this sync group. Choose whether provider sessions should also be deleted.</AlertDialogDescription>
        </AlertDialogHeader>
        <div className="flex flex-col gap-3 rounded-md border p-3 text-sm">
          <input type="hidden" name="group_id" value={group.id} />
          <div className="font-medium">{group.title}</div>
          <div className="break-all font-mono text-xs text-muted-foreground">{group.id}</div>
          <Field orientation="horizontal">
            <Checkbox
              id="delete-provider-sessions-detail"
              name="delete_provider_sessions"
              checked={deleteProviderSessions}
              onCheckedChange={(checked) => setDeleteProviderSessions(checked === true)}
            />
            <FieldContent>
              <FieldLabel htmlFor="delete-provider-sessions-detail">
                <FieldTitle>Delete provider sessions</FieldTitle>
              </FieldLabel>
              <FieldDescription>Also remove the sessions owned by providers in this group.</FieldDescription>
            </FieldContent>
          </Field>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={removeMutation.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={removeMutation.isPending}
            onClick={(event) => {
              event.preventDefault();
              removeMutation.mutate();
            }}
          >
            {removeMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
