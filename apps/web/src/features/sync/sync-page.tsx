import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowRightIcon, GitBranchIcon, RotateCwIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { Link } from "react-router-dom";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
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
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { compactPath, formatDateTime } from "@/lib/format";
import { removeSyncGroup, renameSyncGroup, runSyncGroup } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { SyncGroup, SyncReport } from "@/lib/types";
import { useSyncGroups } from "@/features/sync/queries";

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

export function SyncPage() {
  const syncGroups = useSyncGroups();
  const queryClient = useQueryClient();
  const [renameTarget, setRenameTarget] = useState<SyncGroup | null>(null);
  const [removeTarget, setRemoveTarget] = useState<SyncGroup | null>(null);

  const groups = useMemo(() => syncGroups.data ?? [], [syncGroups.data]);
  const holdings = groups.reduce((sum, group) => sum + group.holdings.length, 0);
  const latestGroups = [...groups].sort((left, right) => right.updated_at - left.updated_at).slice(0, 3);

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

  if (syncGroups.isLoading) return <PageSkeleton />;
  if (syncGroups.error) return <PageError title="Sync groups failed to load" message={syncGroups.error.message} />;

  return (
    <div className="grid gap-5 lg:grid-cols-[18rem_minmax(0,1fr)]" data-sync-list-layout>
      <aside className="flex flex-col gap-4" data-sync-control-panel>
        <Card>
          <CardHeader>
            <CardTitle>Session Sync</CardTitle>
            <CardDescription>Legacy sync group controls and status summary.</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="grid grid-cols-3 gap-2 text-center">
              <div className="rounded-md border p-2">
                <div className="text-lg font-semibold">{groups.length}</div>
                <div className="text-xs text-muted-foreground">Groups</div>
              </div>
              <div className="rounded-md border p-2">
                <div className="text-lg font-semibold">{holdings}</div>
                <div className="text-xs text-muted-foreground">Holdings</div>
              </div>
              <div className="rounded-md border p-2">
                <div className="text-lg font-semibold">{errorCount(groups)}</div>
                <div className="text-xs text-muted-foreground">Errors</div>
              </div>
            </div>

            <Separator />

            <div className="flex flex-col gap-2">
              <Button asChild variant="outline" className="justify-start">
                <Link to="/">
                  Sessions
                  <ArrowRightIcon data-icon="inline-end" />
                </Link>
              </Button>
              <Button variant="outline" className="justify-start" onClick={() => syncGroups.refetch()} disabled={syncGroups.isFetching}>
                {syncGroups.isFetching ? <Spinner data-icon="inline-start" /> : <RotateCwIcon data-icon="inline-start" />}
                Refresh
              </Button>
            </div>

            {latestGroups.length ? (
              <div className="flex flex-col gap-2" data-sync-latest-groups>
                <div className="text-xs font-medium uppercase text-muted-foreground">Recent</div>
                {latestGroups.map((group) => (
                  <Link key={group.id} to={`/sync/${encodeURIComponent(group.id)}`} className="rounded-md border p-2 text-sm hover:bg-accent">
                    <div className="truncate font-medium">{group.title}</div>
                    <div className="truncate text-xs text-muted-foreground">{formatDateTime(group.updated_at)}</div>
                  </Link>
                ))}
              </div>
            ) : null}
          </CardContent>
        </Card>
      </aside>

      <section className="flex min-w-0 flex-col gap-4" data-sync-result-panel>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            <Badge variant="secondary" className="w-fit">Sync Groups</Badge>
            <h1 className="text-2xl font-semibold">Groups</h1>
            <p className="text-sm text-muted-foreground">Rows preserve the legacy View, Sync Latest, Rename, and Remove action placement.</p>
          </div>
        </div>

        {groups.length === 0 ? (
          <PageEmpty title="No sync groups" description="Create a sync group from a session row or detail action." />
        ) : (
          <div className="flex flex-col gap-3" data-sync-row-list>
            {groups.map((group) => {
              const latest = latestHolding(group);
              return (
                <Card key={group.id} data-sync-row>
                  <CardContent className="grid gap-4 p-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
                    <div className="flex min-w-0 flex-col gap-3">
                      <div className="flex flex-wrap items-center gap-2">
                        <GitBranchIcon />
                        <h2 className="truncate text-base font-semibold">{group.title}</h2>
                        {group.source_provider ? <Badge variant="outline">{group.source_provider}</Badge> : null}
                        <Badge variant="secondary">{group.holdings.length} holdings</Badge>
                      </div>
                      <div className="grid gap-2 text-sm text-muted-foreground sm:grid-cols-2">
                        <div className="truncate">{group.id}</div>
                        <div>Updated {formatDateTime(group.updated_at)}</div>
                        <div className="truncate">Latest {latest ? `${latest.provider}:${latest.session_id}` : "-"}</div>
                        <div className="truncate">{latest?.target_dir ? compactPath(latest.target_dir) : "No target dir"}</div>
                      </div>
                      {group.holdings.some((holding) => holding.last_error) ? (
                        <div className="flex flex-wrap gap-2">
                          {group.holdings.filter((holding) => holding.last_error).map((holding) => (
                            <Badge key={holding.id} variant="destructive">
                              {holding.provider}: {holding.last_error}
                            </Badge>
                          ))}
                        </div>
                      ) : null}
                    </div>

                    <div className="flex flex-wrap justify-end gap-2" data-sync-row-actions>
                      <Button asChild variant="outline" size="sm">
                        <Link to={`/sync/${encodeURIComponent(group.id)}`}>
                          View
                          <ArrowRightIcon data-icon="inline-end" />
                        </Link>
                      </Button>
                      <Button variant="outline" size="sm" onClick={() => runMutation.mutate(group)} disabled={runMutation.isPending}>
                        {runMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
                        Sync Latest
                      </Button>
                      <Button variant="outline" size="sm" onClick={() => setRenameTarget(group)}>
                        Rename
                      </Button>
                      <Button variant="destructive" size="sm" onClick={() => setRemoveTarget(group)}>
                        Remove
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              );
            })}
          </div>
        )}
      </section>

      <RenameSyncGroupDialog target={renameTarget} open={Boolean(renameTarget)} onOpenChange={(open) => !open && setRenameTarget(null)} />
      <RemoveSyncGroupDialog target={removeTarget} open={Boolean(removeTarget)} onOpenChange={(open) => !open && setRemoveTarget(null)} />
    </div>
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
        <form className="flex flex-col gap-5" onSubmit={form.handleSubmit((values) => renameMutation.mutate(values))}>
          <input type="hidden" name="group_id" value={target?.id || ""} />
          <FieldGroup>
            <Field data-invalid={Boolean(form.formState.errors.title)}>
              <FieldLabel htmlFor="rename-sync-title">Title</FieldLabel>
              <Input id="rename-sync-title" aria-invalid={Boolean(form.formState.errors.title)} {...form.register("title")} />
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
