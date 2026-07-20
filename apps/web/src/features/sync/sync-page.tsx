import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowRightIcon, CheckIcon, GitBranchIcon, RotateCwIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { Link } from "react-router-dom";
import { toast } from "sonner";
import { z } from "zod";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SectionHeading } from "@/components/shared/section-heading";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
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
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldTitle } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { useManagerMeta, useManagerProviders } from "@/features/manager/queries";
import { useSyncGroups } from "@/features/sync/queries";
import { formatDateTime } from "@/lib/format";
import { removeSyncGroup, renameSyncGroup, runSyncGroup } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ProviderInfo, SyncGroup, SyncReport } from "@/lib/types";

const renameSchema = z.object({
  title: z.string().trim().min(1, "Enter a title."),
});

type RenameForm = z.infer<typeof renameSchema>;

function errorCount(groups: SyncGroup[]) {
  return groups.flatMap((group) => group.holdings).filter((holding) => holding.last_error).length;
}

function latestHolding(group: SyncGroup) {
  return [...group.holdings].sort((left, right) => (right.last_active_at ?? 0) - (left.last_active_at ?? 0))[0];
}

function syncReportDescription(report: SyncReport) {
  const success = report.success.length ? `success=${report.success.length}` : "success=0";
  const errors = report.errors.length ? `errors=${report.errors.length}` : "errors=0";
  return `${report.source_provider} · ${success} · ${errors}`;
}

function providerOptions(providers: ProviderInfo[] | undefined) {
  return (providers ?? []).filter((provider) => provider.scan || provider.export);
}

function matchesSyncGroupSearch(group: SyncGroup, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  const latest = latestHolding(group);
  return [
    group.title,
    group.id,
    group.source_provider,
    latest?.provider,
    latest?.session_id,
    latest?.target_dir,
  ].some((value) => value?.toLowerCase().includes(normalized));
}

function matchesSyncGroupProviders(group: SyncGroup, selectedProviders: string[]) {
  if (!selectedProviders.length) return true;

  const providers = new Set([
    ...(group.source_provider ? [group.source_provider] : []),
    ...group.holdings.map((holding) => holding.provider),
  ]);
  return selectedProviders.some((providerId) => providers.has(providerId));
}

function matchesSyncGroupWorkspace(group: SyncGroup, workspace: string | null | undefined) {
  if (!workspace) return true;
  const dirs = group.holdings.map((holding) => holding.target_dir).filter(Boolean);
  if (!dirs.length) return true;
  return dirs.some((dir) => dir === workspace || dir?.startsWith(`${workspace}/`));
}

function ProviderControls({
  providers,
  selected,
  onToggle,
}: {
  providers: ProviderInfo[];
  selected: string[];
  onToggle: (providerId: string) => void;
}) {
  if (!providers.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No providers</EmptyTitle>
          <EmptyDescription>No sync providers were returned by the backend.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollArea className="min-h-0 flex-1 pr-3" data-sync-provider-controls>
      <div className="flex flex-col gap-2">
        {providers.map((provider) => {
          const checked = selected.length === 0 || selected.includes(provider.id);
          return (
            <SelectableRowButton
              key={provider.id}
              selected={checked}
              leading={<ProviderLogo providerId={provider.id} size="sm" alt={provider.name} />}
              title={provider.name}
              trailing={checked ? <CheckIcon className="text-muted-foreground size-4" aria-hidden /> : null}
              onClick={() => onToggle(provider.id)}
            />
          );
        })}
      </div>
    </ScrollArea>
  );
}

function ControlPanel({
  workspace,
  providers,
  selectedProviders,
  onToggleProvider,
}: {
  workspace: string | null | undefined;
  providers: ProviderInfo[];
  selectedProviders: string[];
  onToggleProvider: (providerId: string) => void;
}) {
  return (
    <PanelCard className="min-h-0" data-sync-control-panel>
      <section className="flex flex-col gap-3 border-b pb-4" data-sync-workspace-summary>
        <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
      </section>
      <ProviderControls providers={providers} selected={selectedProviders} onToggle={onToggleProvider} />
    </PanelCard>
  );
}

function SyncGroupRow({
  group,
  onRename,
  onRemove,
  onSyncLatest,
  syncing,
}: {
  group: SyncGroup;
  onRename: (group: SyncGroup) => void;
  onRemove: (group: SyncGroup) => void;
  onSyncLatest: (group: SyncGroup) => void;
  syncing: boolean;
}) {
  const latest = latestHolding(group);
  const detailHref = `/sync/${encodeURIComponent(group.id)}`;
  const errorHoldings = group.holdings.filter((holding) => holding.last_error);

  return (
    <EntityRow
      data-sync-row
      actionsProps={{ "data-sync-row-actions": true }}
      actions={(
        <>
          <Button asChild variant="outline">
            <Link to={detailHref}>
              View
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          </Button>
          <Button variant="outline" onClick={() => onSyncLatest(group)} disabled={syncing}>
            {syncing ? <Spinner data-icon="inline-start" /> : null}
            Sync Latest
          </Button>
          <Button variant="outline" onClick={() => onRename(group)}>
            Rename
          </Button>
          <Button variant="destructive" onClick={() => onRemove(group)}>
            Remove
          </Button>
        </>
      )}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <Link to={detailHref} className="flex min-w-0 items-center gap-2 truncate text-sm font-medium hover:underline">
          <GitBranchIcon className="size-4 shrink-0" aria-hidden />
          <span className="truncate">{group.title}</span>
        </Link>
        <div className="text-muted-foreground flex flex-wrap gap-2 text-xs">
          {group.source_provider ? <Badge variant="outline">{group.source_provider}</Badge> : null}
          <Badge variant="secondary">{group.holdings.length} holdings</Badge>
          <span>Updated {formatDateTime(group.updated_at)}</span>
        </div>
        <div className="text-muted-foreground flex flex-col gap-1 text-xs">
          <span className="truncate font-mono">{group.id}</span>
          <span className="truncate">
            Latest {latest ? `${latest.provider}:${latest.session_id}` : "-"}
          </span>
          <PathText value={latest?.target_dir} fallback="No target dir" wrap="all" />
        </div>
        {errorHoldings.length ? (
          <div className="flex flex-wrap gap-2">
            {errorHoldings.map((holding) => (
              <Badge key={holding.id} variant="destructive">
                {holding.provider}: {holding.last_error}
              </Badge>
            ))}
          </div>
        ) : null}
      </div>
    </EntityRow>
  );
}

function ResultPanel({
  groups,
  filteredGroups,
  search,
  onSearchChange,
  onRefresh,
  refreshing,
  onRename,
  onRemove,
  onSyncLatest,
  syncing,
}: {
  groups: SyncGroup[];
  filteredGroups: SyncGroup[];
  search: string;
  onSearchChange: (value: string) => void;
  onRefresh: () => void;
  refreshing: boolean;
  onRename: (group: SyncGroup) => void;
  onRemove: (group: SyncGroup) => void;
  onSyncLatest: (group: SyncGroup) => void;
  syncing: boolean;
}) {
  const holdings = groups.reduce((sum, group) => sum + group.holdings.length, 0);
  const searchActive = search.trim().length > 0;
  const summary = searchActive
    ? `${filteredGroups.length} shown / ${groups.length} total`
    : `${groups.length} sync groups / ${holdings} holdings`;

  return (
    <PanelCard variant="plain" className="grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4" data-sync-result-panel>
      <MetricGrid columns="three">
        <MetricTile label="Groups" value={groups.length} variant="compact" />
        <MetricTile label="Holdings" value={holdings} variant="compact" />
        <MetricTile label="Errors" value={errorCount(groups)} variant="compact" />
      </MetricGrid>
      <Separator />
      <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3">
        <div className="flex flex-col gap-3 border-b pb-2">
          <SectionHeading
            variant="page"
            titleAs="h1"
            eyebrow="Sync Groups"
            title="Shared Sessions"
            description={summary}
            className="border-b-0 pb-0"
            actions={(
              <Button variant="outline" onClick={onRefresh} disabled={refreshing}>
                {refreshing ? <Spinner data-icon="inline-start" /> : <RotateCwIcon data-icon="inline-start" />}
                Refresh
              </Button>
            )}
          />
          <div className="relative w-full max-w-sm">
            <SearchIcon className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <Input
              className="pl-8"
              value={search}
              onChange={(event) => onSearchChange(event.target.value)}
              placeholder="Search title, id, provider, or path"
              data-sync-preview-search
            />
          </div>
        </div>
        <ScrollArea className="min-h-0 pr-3">
          {filteredGroups.length === 0 ? (
            <PageEmpty
              title={groups.length ? "No matching sync groups" : "No sync groups"}
              description={
                groups.length
                  ? "Try another search or provider filter."
                  : "Create a sync group from a session row or detail action."
              }
            />
          ) : (
            <div className="flex flex-col gap-2" data-sync-row-list>
              {filteredGroups.map((group) => (
                <SyncGroupRow
                  key={group.id}
                  group={group}
                  onRename={onRename}
                  onRemove={onRemove}
                  onSyncLatest={onSyncLatest}
                  syncing={syncing}
                />
              ))}
            </div>
          )}
        </ScrollArea>
      </div>
    </PanelCard>
  );
}

export function SyncPage() {
  const syncGroups = useSyncGroups();
  const meta = useManagerMeta();
  const providers = useManagerProviders();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [selectedProviders, setSelectedProviders] = useState<string[]>([]);
  const [renameTarget, setRenameTarget] = useState<SyncGroup | null>(null);
  const [removeTarget, setRemoveTarget] = useState<SyncGroup | null>(null);

  const groups = useMemo(() => syncGroups.data ?? [], [syncGroups.data]);
  const filteredGroups = useMemo(
    () =>
      groups.filter(
        (group) =>
          matchesSyncGroupProviders(group, selectedProviders) &&
          matchesSyncGroupWorkspace(group, meta.data?.selected_workspace) &&
          matchesSyncGroupSearch(group, search),
      ),
    [groups, meta.data?.selected_workspace, search, selectedProviders],
  );

  const runMutation = useMutation({
    mutationFn: (group: SyncGroup) => runSyncGroup({ group_id: group.id }),
    onSuccess: async (report, group) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(group.id) }),
      ]);
      toast.success("Sync Latest", { description: syncReportDescription(report) });
    },
  });

  if (syncGroups.isLoading || meta.isLoading || providers.isLoading) return <PageSkeleton />;
  if (syncGroups.error) return <PageError title="Sync groups failed to load" message={syncGroups.error.message} />;
  if (meta.error) return <PageError title="Workspace failed to load" message={meta.error.message} />;
  if (providers.error) return <PageError title="Providers failed to load" message={providers.error.message} />;

  const options = providerOptions(providers.data);

  function toggleProvider(providerId: string) {
    setSelectedProviders((current) =>
      current.includes(providerId) ? current.filter((id) => id !== providerId) : [...current, providerId],
    );
    setSearch("");
  }

  return (
    <>
      <TwoPanePage data-sync-list-layout>
        <ControlPanel
          workspace={meta.data?.selected_workspace}
          providers={options}
          selectedProviders={selectedProviders}
          onToggleProvider={toggleProvider}
        />
        <ResultPanel
          groups={groups}
          filteredGroups={filteredGroups}
          search={search}
          onSearchChange={setSearch}
          onRefresh={() => syncGroups.refetch()}
          refreshing={syncGroups.isFetching}
          onRename={setRenameTarget}
          onRemove={setRemoveTarget}
          onSyncLatest={(group) => runMutation.mutate(group)}
          syncing={runMutation.isPending}
        />
      </TwoPanePage>

      <RenameSyncGroupDialog target={renameTarget} open={Boolean(renameTarget)} onOpenChange={(open) => !open && setRenameTarget(null)} />
      <RemoveSyncGroupDialog target={removeTarget} open={Boolean(removeTarget)} onOpenChange={(open) => !open && setRemoveTarget(null)} />
    </>
  );
}

function RenameSyncGroupDialog({
  target,
  open,
  onOpenChange,
}: {
  target: SyncGroup | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const form = useForm<RenameForm>({
    resolver: zodResolver(renameSchema),
    defaultValues: { title: "" },
  });

  useEffect(() => {
    if (!open || !target) return;
    form.reset({ title: target.title });
  }, [form, open, target]);

  const renameMutation = useMutation({
    mutationFn: (values: RenameForm) => {
      if (!target) throw new Error("Missing sync group target");
      return renameSyncGroup(target.id, { title: values.title.trim() });
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups }),
        target ? queryClient.invalidateQueries({ queryKey: queryKeys.syncGroup(target.id) }) : Promise.resolve(),
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
          <DialogDescription>Update the sync group title from the legacy row action.</DialogDescription>
        </DialogHeader>
        <DialogForm onSubmit={form.handleSubmit((values) => renameMutation.mutate(values))}>
          <input type="hidden" name="group_id" value={target?.id || ""} />
          <FieldGroup>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="rename-sync-title">Title</FieldLabel>
              <Input id="rename-sync-title" aria-invalid={Boolean(form.formState.errors.title)} {...form.register("title")} />
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

function RemoveSyncGroupDialog({
  target,
  open,
  onOpenChange,
}: {
  target: SyncGroup | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [deleteProviderSessions, setDeleteProviderSessions] = useState(false);

  const removeMutation = useMutation({
    mutationFn: () => {
      if (!target) throw new Error("Missing sync group target");
      return removeSyncGroup(target.id, deleteProviderSessions);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.syncGroups });
      toast.success("Removed", { description: target?.title });
      onOpenChange(false);
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
        <div className="flex flex-col gap-3 rounded-md border p-3">
          <input type="hidden" name="group_id" value={target?.id || ""} />
          <div className="font-medium">{target?.title || "Sync group"}</div>
          <div className="break-all font-mono text-xs text-muted-foreground">{target?.id || "-"}</div>
          <Field orientation="horizontal">
            <Checkbox
              id="delete-provider-sessions"
              name="delete_provider_sessions"
              checked={deleteProviderSessions}
              onCheckedChange={(checked) => setDeleteProviderSessions(checked === true)}
            />
            <FieldContent>
              <FieldLabel htmlFor="delete-provider-sessions">
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
            disabled={!target || removeMutation.isPending}
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
