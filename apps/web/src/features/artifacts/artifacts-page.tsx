import { useMemo, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArchiveRestoreIcon,
  DatabaseBackupIcon,
  FileCheck2Icon,
  FileOutputIcon,
  RefreshCwIcon,
  SearchIcon,
  ShieldAlertIcon,
  Trash2Icon,
} from "lucide-react";
import { toast } from "sonner";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { SectionHeading } from "@/components/shared/section-heading";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useArtifactInspection,
  useBackup,
  useBackups,
  useCleanupArtifacts,
  useRestoreBackup,
} from "@/features/artifacts/queries";
import { formatBytes, formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type {
  ArtifactCleanupReport,
  ArtifactInspectionEntry,
  ArtifactManifestKind,
  ArtifactRetentionState,
  ArtifactVerificationStatus,
  BackupRestoreStatus,
  BackupView,
  OrphanArtifactFile,
} from "@/lib/types";
import { cn } from "@/lib/utils";

type StorageView = "artifacts" | "backups" | "exports";

const artifactKinds: Array<ArtifactManifestKind | "all"> = [
  "all",
  "event_payload",
  "database_backup",
  "session_backup",
  "session_export",
  "compression_archive",
];

const verificationStates: Array<ArtifactVerificationStatus | "all"> = [
  "all",
  "verified",
  "changed",
  "missing",
  "unverifiable",
];

const retentionStates: Array<ArtifactRetentionState | "all"> = [
  "all",
  "current_event_payload",
  "detached_event_payload",
  "retained",
];

function readable(value: string) {
  return value.replaceAll("_", " ");
}

function statusVariant(status: ArtifactVerificationStatus) {
  if (status === "verified") return "secondary" as const;
  if (status === "missing" || status === "changed") return "destructive" as const;
  return "outline" as const;
}

function restoreVariant(status: BackupRestoreStatus | null | undefined) {
  if (status === "success") return "secondary" as const;
  if (status === "failed") return "destructive" as const;
  return "outline" as const;
}

function MetadataBlock({ value }: { value: Record<string, unknown> }) {
  return (
    <pre className="max-h-52 overflow-auto rounded-md border bg-muted/30 p-3 font-mono text-xs whitespace-pre-wrap break-all">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function DetailLine({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid min-w-0 gap-1 border-b pb-2 last:border-b-0 last:pb-0 sm:grid-cols-[9rem_minmax(0,1fr)]">
      <span className="text-muted-foreground font-mono text-xs uppercase">{label}</span>
      <div className="min-w-0 text-sm">{children || "-"}</div>
    </div>
  );
}

function ManifestDetail({ entry }: { entry: ArtifactInspectionEntry }) {
  const { manifest, verification, retention_state: retentionState } = entry;
  const sessionHref = manifest.provider_id && manifest.provider_session_id
    ? `/sessions/${encodeURIComponent(manifest.provider_id)}/${encodeURIComponent(manifest.provider_session_id)}`
    : null;

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <SectionHeading
        title={manifest.id}
        description={readable(manifest.artifact_kind)}
        actions={(
          <>
            <Badge variant={statusVariant(verification.status)}>{verification.status}</Badge>
            <Badge variant="outline">{readable(retentionState)}</Badge>
          </>
        )}
      />
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-3">
          <DetailLine label="Path"><PathText value={manifest.path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label="Storage">{manifest.storage_kind}</DetailLine>
          <DetailLine label="Size">{formatBytes(manifest.byte_size)}</DetailLine>
          <DetailLine label="Created">{formatDateTime(manifest.created_at_ms)}</DetailLine>
          <DetailLine label="Format">{manifest.format || "-"}</DetailLine>
          <DetailLine label="MIME">{manifest.mime_type || "-"}</DetailLine>
          <DetailLine label="Provider">{manifest.provider_id || "-"}</DetailLine>
          <DetailLine label="Provider session">
            {sessionHref ? <Link className="break-all underline underline-offset-4" to={sessionHref}>{manifest.provider_session_id}</Link> : manifest.provider_session_id || "-"}
          </DetailLine>
          <DetailLine label="Canonical session">{manifest.session_id || "-"}</DetailLine>
          <DetailLine label="Operation">{manifest.operation_id || "-"}</DetailLine>
          <DetailLine label="Projection report">{manifest.projection_report_id || "-"}</DetailLine>
          <DetailLine label="Event">{manifest.event_id || "-"}</DetailLine>
          <DetailLine label="Block">{manifest.block_id || "-"}</DetailLine>
          <DetailLine label="Expected hash"><span className="break-all font-mono text-xs">{verification.expected_content_hash}</span></DetailLine>
          <DetailLine label="Actual hash"><span className="break-all font-mono text-xs">{verification.actual_content_hash || "-"}</span></DetailLine>
          <DetailLine label="Actual size">{verification.actual_byte_size == null ? "-" : formatBytes(verification.actual_byte_size)}</DetailLine>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">Metadata</span>
            <MetadataBlock value={manifest.metadata} />
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}

function ManifestRow({
  entry,
  selected,
  onSelect,
}: {
  entry: ArtifactInspectionEntry;
  selected: boolean;
  onSelect: () => void;
}) {
  const manifest = entry.manifest;
  return (
    <EntityRow
      variant="inline"
      selected={selected}
      className="cursor-pointer"
      onClick={onSelect}
      actions={(
        <Badge variant={statusVariant(entry.verification.status)}>
          {entry.verification.status}
        </Badge>
      )}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex min-w-0 items-center gap-2">
          <strong className="truncate text-sm font-medium">{manifest.provider_session_id || manifest.id}</strong>
          <Badge variant="outline">{manifest.format || manifest.storage_kind}</Badge>
        </div>
        <span className="truncate font-mono text-xs text-muted-foreground">{manifest.path}</span>
        <span className="text-xs text-muted-foreground">
          {formatBytes(manifest.byte_size)} · {formatDateTime(manifest.created_at_ms)} · {readable(entry.retention_state)}
        </span>
      </div>
    </EntityRow>
  );
}

function CleanupReportView({ report }: { report: ArtifactCleanupReport }) {
  return (
    <div className="grid gap-2 border-t pt-3 text-sm sm:grid-cols-3">
      <div>
        <span className="text-muted-foreground block text-xs">Manifests</span>
        <strong>{report.applied ? report.deleted_manifest_ids.length : report.candidate_manifest_ids.length}</strong>
      </div>
      <div>
        <span className="text-muted-foreground block text-xs">Files</span>
        <strong>{report.applied ? report.deleted_paths.length : report.candidate_orphan_paths.length}</strong>
      </div>
      <div>
        <span className="text-muted-foreground block text-xs">Failures</span>
        <strong className={cn(report.failures.length && "text-destructive")}>{report.failures.length}</strong>
      </div>
      {report.failures.length ? (
        <div className="sm:col-span-3">
          {report.failures.map((failure, index) => (
            <p key={`${failure.path || "failure"}-${index}`} className="text-destructive break-words text-xs">
              {failure.path ? `${failure.path}: ` : ""}{failure.reason}
            </p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function CleanupControls() {
  const [retentionHours, setRetentionHours] = useState("168");
  const [confirmOpen, setConfirmOpen] = useState(false);
  const cleanup = useCleanupArtifacts();
  const hours = Number(retentionHours);
  const valid = Number.isInteger(hours) && hours > 0;

  function run(apply: boolean) {
    cleanup.mutate(
      { retention_hours: hours, apply },
      {
        onSuccess: (report) => {
          if (apply) {
            setConfirmOpen(false);
            toast.success("Artifact cleanup completed", {
              description: `${report.deleted_manifest_ids.length} manifests and ${report.deleted_paths.length} files removed`,
            });
          }
        },
        onError: (error) => toast.error(apply ? "Artifact cleanup failed" : "Cleanup plan failed", { description: error.message }),
      },
    );
  }

  return (
    <section className="flex flex-col gap-3 border-b pb-4">
      <SectionHeading
        title="Event payload retention"
        description="Only detached, verified memorph-managed event payloads and valid managed-layout orphans are eligible."
      />
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex min-w-36 flex-1 flex-col gap-1 text-xs text-muted-foreground">
          Retention hours
          <Input
            inputMode="numeric"
            min={1}
            value={retentionHours}
            onChange={(event) => setRetentionHours(event.target.value)}
          />
        </label>
        <Button type="button" variant="outline" disabled={!valid || cleanup.isPending} onClick={() => run(false)}>
          {cleanup.isPending ? <Spinner data-icon="inline-start" /> : <SearchIcon data-icon="inline-start" />}
          Preview
        </Button>
        <Button type="button" variant="destructive" disabled={!valid || cleanup.isPending} onClick={() => setConfirmOpen(true)}>
          <Trash2Icon data-icon="inline-start" />
          Apply
        </Button>
      </div>
      {cleanup.data ? <CleanupReportView report={cleanup.data} /> : null}
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia><ShieldAlertIcon /></AlertDialogMedia>
            <AlertDialogTitle>Apply artifact cleanup?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes eligible detached manifests and managed orphan files older than {hours} hours. Changed, missing, shared, backup, export, and compression artifacts remain untouched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={cleanup.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={!valid || cleanup.isPending}
              onClick={(event) => {
                event.preventDefault();
                run(true);
              }}
            >
              {cleanup.isPending ? <Spinner data-icon="inline-start" /> : <Trash2Icon data-icon="inline-start" />}
              Apply cleanup
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

function OrphanRows({ files }: { files: OrphanArtifactFile[] }) {
  if (!files.length) return null;
  return (
    <section className="flex flex-col gap-2 border-t pt-3">
      <SectionHeading title="Unregistered files" badge={files.length} />
      {files.map((file) => (
        <EntityRow
          key={file.path}
          variant="inline"
          actions={<Badge variant={file.managed_layout ? "outline" : "destructive"}>{file.managed_layout ? "managed layout" : "malformed layout"}</Badge>}
        >
          <div className="flex min-w-0 flex-col gap-1">
            <PathText value={file.path} tone="default" wrap="all" />
            <span className="text-xs text-muted-foreground">{formatBytes(file.byte_size)} · modified {formatDateTime(file.modified_at_ms)}</span>
          </div>
        </EntityRow>
      ))}
    </section>
  );
}

function ArtifactsView({ exportsOnly = false }: { exportsOnly?: boolean }) {
  const inspection = useArtifactInspection();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [kind, setKind] = useState<ArtifactManifestKind | "all">(exportsOnly ? "session_export" : "all");
  const [verification, setVerification] = useState<ArtifactVerificationStatus | "all">("all");
  const [retention, setRetention] = useState<ArtifactRetentionState | "all">("all");
  const [search, setSearch] = useState("");

  const rows = useMemo(() => {
    const normalized = search.trim().toLowerCase();
    return (inspection.data?.registered ?? []).filter((entry) => {
      if (exportsOnly && entry.manifest.artifact_kind !== "session_export") return false;
      if (!exportsOnly && kind !== "all" && entry.manifest.artifact_kind !== kind) return false;
      if (verification !== "all" && entry.verification.status !== verification) return false;
      if (retention !== "all" && entry.retention_state !== retention) return false;
      if (!normalized) return true;
      const manifest = entry.manifest;
      return [
        manifest.id,
        manifest.path,
        manifest.provider_id,
        manifest.provider_session_id,
        manifest.session_id,
        manifest.operation_id,
        manifest.projection_report_id,
      ].some((value) => value?.toLowerCase().includes(normalized));
    });
  }, [exportsOnly, inspection.data?.registered, kind, retention, search, verification]);

  if (inspection.isLoading) return <PageSkeleton />;
  if (inspection.error) return <PageError title="Artifact inspection failed" message={inspection.error.message} />;
  if (!inspection.data) return <PageEmpty title="No inspection data" description="The artifact inspection returned no report." />;

  const selected = rows.find((entry) => entry.manifest.id === selectedId) ?? rows[0] ?? null;
  return (
    <div className="grid gap-4 xl:min-h-0 xl:flex-1 xl:grid-cols-[minmax(360px,0.85fr)_minmax(0,1.15fr)]">
      <section className="flex min-w-0 flex-col gap-3 border-r-0 xl:min-h-0 xl:border-r xl:pr-4">
        {!exportsOnly ? <CleanupControls /> : null}
        <div className="grid gap-2 sm:grid-cols-2">
          {!exportsOnly ? (
            <Select value={kind} onValueChange={(value) => setKind(value as ArtifactManifestKind | "all")}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>{artifactKinds.map((value) => <SelectItem key={value} value={value}>{readable(value)}</SelectItem>)}</SelectContent>
            </Select>
          ) : null}
          <Select value={verification} onValueChange={(value) => setVerification(value as ArtifactVerificationStatus | "all")}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>{verificationStates.map((value) => <SelectItem key={value} value={value}>{readable(value)}</SelectItem>)}</SelectContent>
          </Select>
          {!exportsOnly ? (
            <Select value={retention} onValueChange={(value) => setRetention(value as ArtifactRetentionState | "all")}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>{retentionStates.map((value) => <SelectItem key={value} value={value}>{readable(value)}</SelectItem>)}</SelectContent>
            </Select>
          ) : null}
          <div className={cn("relative", exportsOnly && "sm:col-span-1")}>
            <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input className="pl-9" placeholder="Path, session, operation..." value={search} onChange={(event) => setSearch(event.target.value)} />
          </div>
        </div>
        <ScrollArea className="h-[min(28rem,60vh)] pr-3 xl:min-h-0 xl:flex-1">
          <div className="flex flex-col">
            {rows.length ? rows.map((entry) => (
              <ManifestRow
                key={entry.manifest.id}
                entry={entry}
                selected={selected?.manifest.id === entry.manifest.id}
                onSelect={() => setSelectedId(entry.manifest.id)}
              />
            )) : <PageEmpty title={exportsOnly ? "No exports" : "No matching artifacts"} description="Adjust the current filters or create a new artifact." />}
            {!exportsOnly ? <OrphanRows files={inspection.data.orphan_files} /> : null}
          </div>
        </ScrollArea>
      </section>
      <section className={cn("min-w-0 border-t pt-4 xl:min-h-0 xl:border-t-0 xl:pt-0", !selected && "hidden xl:block")}>
        {selected ? <ManifestDetail entry={selected} /> : (
          <PageEmpty title={exportsOnly ? "No export selected" : "No artifact selected"} description="Choose a manifest from the list to inspect integrity and provenance." />
        )}
      </section>
    </div>
  );
}

function BackupDetail({
  view,
  onRestore,
}: {
  view: BackupView;
  onRestore: (view: BackupView) => void;
}) {
  const backup = view.entry.backup;
  const latest = view.entry.latest_restore;
  const sessionHref = backup.provider_id && backup.provider_session_id
    ? `/sessions/${encodeURIComponent(backup.provider_id)}/${encodeURIComponent(backup.provider_session_id)}`
    : null;

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <SectionHeading
        title={backup.id}
        description={backup.provider_id || "Provider identity unavailable"}
        actions={(
          <>
            <Badge variant={statusVariant(view.verification.status)}>{view.verification.status}</Badge>
            <Button type="button" disabled={view.verification.status !== "verified"} onClick={() => onRestore(view)}>
              <ArchiveRestoreIcon data-icon="inline-start" />
              Restore
            </Button>
          </>
        )}
      />
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-3">
          <DetailLine label="Artifact path"><PathText value={backup.artifact.path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label="Source path"><PathText value={backup.source_path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label="Provider">{backup.provider_id || "-"}</DetailLine>
          <DetailLine label="Provider session">
            {sessionHref ? <Link className="break-all underline underline-offset-4" to={sessionHref}>{backup.provider_session_id}</Link> : backup.provider_session_id || "-"}
          </DetailLine>
          <DetailLine label="Canonical session">{backup.session_id || "-"}</DetailLine>
          <DetailLine label="Operation">{backup.operation_id || "-"}</DetailLine>
          <DetailLine label="Created">{formatDateTime(backup.created_at_ms)}</DetailLine>
          <DetailLine label="Size">{formatBytes(backup.artifact.byte_size)}</DetailLine>
          <DetailLine label="Format">{backup.artifact.format || "-"}</DetailLine>
          <DetailLine label="Hash"><span className="break-all font-mono text-xs">{backup.artifact.content_hash}</span></DetailLine>
          <DetailLine label="Restore status">
            {latest ? <Badge variant={restoreVariant(latest.status)}>{latest.status}</Badge> : "Never restored"}
          </DetailLine>
          <DetailLine label="Restore actor">{latest?.actor || "-"}</DetailLine>
          <DetailLine label="Restore started">{latest ? formatDateTime(latest.started_at_ms) : "-"}</DetailLine>
          <DetailLine label="Restore finished">{latest?.finished_at_ms ? formatDateTime(latest.finished_at_ms) : "-"}</DetailLine>
          <DetailLine label="Restore error"><span className={cn(latest?.error && "text-destructive")}>{latest?.error || "-"}</span></DetailLine>
          <DetailLine label="Restore hint">{backup.restore_hint || "-"}</DetailLine>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">Restore metadata</span>
            <MetadataBlock value={backup.metadata} />
          </div>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">Artifact metadata</span>
            <MetadataBlock value={backup.artifact.metadata} />
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}

function RestoreBackupDialog({
  target,
  open,
  onOpenChange,
}: {
  target: BackupView | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const restore = useRestoreBackup();
  const backup = target?.entry.backup;

  function applyRestore() {
    if (!backup) return;
    restore.mutate(backup.id, {
      onSuccess: (record) => {
        onOpenChange(false);
        toast.success("Native backup restored", { description: `${backup.provider_id || "provider"} · ${record.id}` });
      },
      onError: (error) => toast.error("Backup restore failed", { description: error.message }),
    });
  }

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogMedia><DatabaseBackupIcon /></AlertDialogMedia>
          <AlertDialogTitle>Restore provider-native backup?</AlertDialogTitle>
          <AlertDialogDescription>
            This invokes the provider-owned restore contract for {backup?.provider_id || "the provider"} session {backup?.provider_session_id || "-"}. Integrity and identity are verified again before any provider data is written.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="rounded-md border p-3">
          <PathText value={backup?.artifact.path} tone="default" wrap="all" />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={restore.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={!backup || restore.isPending}
            onClick={(event) => {
              event.preventDefault();
              applyRestore();
            }}
          >
            {restore.isPending ? <Spinner data-icon="inline-start" /> : <ArchiveRestoreIcon data-icon="inline-start" />}
            Restore
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function BackupsView() {
  const [provider, setProvider] = useState("");
  const [providerSessionId, setProviderSessionId] = useState("");
  const [restoreStatus, setRestoreStatus] = useState<BackupRestoreStatus | "all">("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<BackupView | null>(null);
  const params = useMemo(() => ({
    provider: provider.trim() || undefined,
    provider_session_id: providerSessionId.trim() || undefined,
    restore_status: restoreStatus === "all" ? undefined : restoreStatus,
    limit: 500,
  }), [provider, providerSessionId, restoreStatus]);
  const backups = useBackups(params);
  const rows = backups.data ?? [];
  const selectedSummary = rows.find((view) => view.entry.backup.id === selectedId) ?? rows[0] ?? null;
  const detail = useBackup(selectedSummary?.entry.backup.id ?? null);
  const selected = detail.data ?? selectedSummary;

  if (backups.isLoading) return <PageSkeleton />;
  if (backups.error) return <PageError title="Backups failed to load" message={backups.error.message} />;

  return (
    <>
      <div className="grid gap-4 xl:min-h-0 xl:flex-1 xl:grid-cols-[minmax(360px,0.85fr)_minmax(0,1.15fr)]">
        <section className="flex min-w-0 flex-col gap-3 border-r-0 xl:min-h-0 xl:border-r xl:pr-4">
          <div className="grid gap-2 sm:grid-cols-2">
            <Input placeholder="Provider" value={provider} onChange={(event) => setProvider(event.target.value)} />
            <Input placeholder="Provider session ID" value={providerSessionId} onChange={(event) => setProviderSessionId(event.target.value)} />
            <Select value={restoreStatus} onValueChange={(value) => setRestoreStatus(value as BackupRestoreStatus | "all")}>
              <SelectTrigger className="sm:col-span-2"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">all restore states</SelectItem>
                <SelectItem value="success">success</SelectItem>
                <SelectItem value="failed">failed</SelectItem>
                <SelectItem value="running">running</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <ScrollArea className="h-[min(28rem,60vh)] pr-3 xl:min-h-0 xl:flex-1">
            <div className="flex flex-col">
              {rows.length ? rows.map((view) => {
                const backup = view.entry.backup;
                const latest = view.entry.latest_restore;
                return (
                  <EntityRow
                    key={backup.id}
                    variant="inline"
                    selected={selectedSummary?.entry.backup.id === backup.id}
                    className="cursor-pointer"
                    onClick={() => setSelectedId(backup.id)}
                    actions={<Badge variant={statusVariant(view.verification.status)}>{view.verification.status}</Badge>}
                  >
                    <div className="flex min-w-0 flex-col gap-1">
                      <strong className="truncate text-sm font-medium">{backup.provider_session_id || backup.id}</strong>
                      <span className="truncate font-mono text-xs text-muted-foreground">{backup.artifact.path}</span>
                      <span className="text-xs text-muted-foreground">
                        {backup.provider_id || "-"} · {formatBytes(backup.artifact.byte_size)} · {latest ? `restore ${latest.status}` : "never restored"}
                      </span>
                    </div>
                  </EntityRow>
                );
              }) : <PageEmpty title="No backups" description="No registered backups match these filters." />}
            </div>
          </ScrollArea>
        </section>
        <section className={cn("min-w-0 border-t pt-4 xl:min-h-0 xl:border-t-0 xl:pt-0", !selected && "hidden xl:block")}>
          {detail.isLoading ? <PageSkeleton /> : detail.error ? (
            <PageError title="Backup detail failed to load" message={detail.error.message} />
          ) : selected ? (
            <BackupDetail view={selected} onRestore={setRestoreTarget} />
          ) : <PageEmpty title="No backup selected" description="Choose a registered backup to inspect its native restore contract." />}
        </section>
      </div>
      <RestoreBackupDialog
        target={restoreTarget}
        open={Boolean(restoreTarget)}
        onOpenChange={(open) => {
          if (!open) setRestoreTarget(null);
        }}
      />
    </>
  );
}

export function ArtifactsPage() {
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedView = searchParams.get("view");
  const view: StorageView = requestedView === "backups" || requestedView === "exports" ? requestedView : "artifacts";
  const inspection = useArtifactInspection();
  const backups = useBackups({ limit: 500 });
  const entries = inspection.data?.registered ?? [];
  const verified = entries.filter((entry) => entry.verification.status === "verified").length;
  const attention = entries.length - verified;
  const exports = entries.filter((entry) => entry.manifest.artifact_kind === "session_export").length;

  function setView(next: string) {
    setSearchParams(next === "artifacts" ? {} : { view: next });
  }

  return (
    <PanelCard
      variant="plain"
      className="flex h-full min-h-0 min-w-0 flex-col gap-4 overflow-y-auto xl:overflow-hidden"
      data-artifacts-page
    >
      <SectionHeading
        title="Storage Registry"
        description="SQLite manifests and restore records for memorph-managed artifacts. Provider sources remain provider-owned."
        actions={(
          <Button
            type="button"
            variant="outline"
            disabled={inspection.isFetching || backups.isFetching}
            onClick={() => {
              queryClient.invalidateQueries({ queryKey: queryKeys.artifacts });
              queryClient.invalidateQueries({ queryKey: ["backups"] });
            }}
          >
            {inspection.isFetching || backups.isFetching ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
            Refresh
          </Button>
        )}
      />
      <MetricGrid columns="auto" className="grid-cols-2">
        <MetricTile label="Registered" value={entries.length} hint="artifact manifests" variant="compact" />
        <MetricTile label="Verified" value={verified} hint="content matches" variant="compact" />
        <MetricTile label="Attention" value={attention + (inspection.data?.orphan_files.length ?? 0)} hint="integrity or orphan" variant="compact" />
        <MetricTile label="Backups / Exports" value={`${backups.data?.length ?? 0} / ${exports}`} hint="queryable records" variant="compact" />
      </MetricGrid>
      <Tabs value={view} onValueChange={setView} className="min-w-0 xl:min-h-0 xl:flex-1">
        <TabsList>
          <TabsTrigger value="artifacts"><FileCheck2Icon />Artifacts</TabsTrigger>
          <TabsTrigger value="backups"><DatabaseBackupIcon />Backups</TabsTrigger>
          <TabsTrigger value="exports"><FileOutputIcon />Exports</TabsTrigger>
        </TabsList>
        <Separator />
        <div className="flex min-w-0 flex-col xl:min-h-0 xl:flex-1">
          {view === "artifacts" ? <ArtifactsView /> : null}
          {view === "backups" ? <BackupsView /> : null}
          {view === "exports" ? <ArtifactsView exportsOnly /> : null}
        </div>
      </Tabs>
    </PanelCard>
  );
}
