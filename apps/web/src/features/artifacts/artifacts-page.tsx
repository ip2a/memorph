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
import { useI18n } from "@/lib/i18n-context";

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

type Translate = ReturnType<typeof useI18n>["t"];

function artifactKindLabel(value: ArtifactManifestKind | "all", t: Translate) {
  const keys = { all: "artifactKindAll", event_payload: "artifactKindEventPayload", database_backup: "artifactKindDatabaseBackup", session_backup: "artifactKindSessionBackup", session_export: "artifactKindSessionExport", compression_archive: "artifactKindCompressionArchive" } as const;
  return t(keys[value]);
}

function verificationLabel(value: ArtifactVerificationStatus | "all", t: Translate) {
  const keys = { all: "artifactStatusAll", verified: "artifactStatusVerified", changed: "artifactStatusChanged", missing: "artifactStatusMissing", unverifiable: "artifactStatusUnverifiable" } as const;
  return t(keys[value]);
}

function retentionLabel(value: ArtifactRetentionState | "all", t: Translate) {
  const keys = { all: "artifactRetentionAll", current_event_payload: "artifactRetentionCurrentEventPayload", detached_event_payload: "artifactRetentionDetachedEventPayload", retained: "artifactRetentionRetained" } as const;
  return t(keys[value]);
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
  const { t } = useI18n();
  const { manifest, verification, retention_state: retentionState } = entry;
  const sessionHref = manifest.provider_id && manifest.provider_session_id
    ? `/sessions/${encodeURIComponent(manifest.provider_id)}/${encodeURIComponent(manifest.provider_session_id)}`
    : null;

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <SectionHeading
        title={manifest.id}
        description={artifactKindLabel(manifest.artifact_kind, t)}
        actions={(
          <>
            <Badge variant={statusVariant(verification.status)}>{verificationLabel(verification.status, t)}</Badge>
            <Badge variant="outline">{retentionLabel(retentionState, t)}</Badge>
          </>
        )}
      />
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-3">
          <DetailLine label={t("artifactPath")}><PathText value={manifest.path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label={t("artifactStorage")}>{manifest.storage_kind}</DetailLine>
          <DetailLine label={t("size")}>{formatBytes(manifest.byte_size)}</DetailLine>
          <DetailLine label={t("artifactCreated")}>{formatDateTime(manifest.created_at_ms)}</DetailLine>
          <DetailLine label={t("artifactFormat")}>{manifest.format || "-"}</DetailLine>
          <DetailLine label={t("artifactMime")}>{manifest.mime_type || "-"}</DetailLine>
          <DetailLine label={t("provider")}>{manifest.provider_id || "-"}</DetailLine>
          <DetailLine label={t("artifactProviderSession")}>
            {sessionHref ? <Link className="break-all underline underline-offset-4" to={sessionHref}>{manifest.provider_session_id}</Link> : manifest.provider_session_id || "-"}
          </DetailLine>
          <DetailLine label={t("artifactCanonicalSession")}>{manifest.session_id || "-"}</DetailLine>
          <DetailLine label={t("artifactOperation")}>{manifest.operation_id || "-"}</DetailLine>
          <DetailLine label={t("artifactProjectionReport")}>{manifest.projection_report_id || "-"}</DetailLine>
          <DetailLine label={t("artifactEvent")}>{manifest.event_id || "-"}</DetailLine>
          <DetailLine label={t("artifactBlock")}>{manifest.block_id || "-"}</DetailLine>
          <DetailLine label={t("artifactExpectedHash")}><span className="break-all font-mono text-xs">{verification.expected_content_hash}</span></DetailLine>
          <DetailLine label={t("artifactActualHash")}><span className="break-all font-mono text-xs">{verification.actual_content_hash || "-"}</span></DetailLine>
          <DetailLine label={t("artifactActualSize")}>{verification.actual_byte_size == null ? "-" : formatBytes(verification.actual_byte_size)}</DetailLine>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">{t("artifactMetadata")}</span>
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
  const { t } = useI18n();
  const manifest = entry.manifest;
  return (
    <EntityRow
      variant="inline"
      selected={selected}
      className="cursor-pointer"
      onClick={onSelect}
      actions={(
        <Badge variant={statusVariant(entry.verification.status)}>
          {verificationLabel(entry.verification.status, t)}
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
          {formatBytes(manifest.byte_size)} · {formatDateTime(manifest.created_at_ms)} · {retentionLabel(entry.retention_state, t)}
        </span>
      </div>
    </EntityRow>
  );
}

function CleanupReportView({ report }: { report: ArtifactCleanupReport }) {
  const { t } = useI18n();
  return (
    <div className="grid gap-2 border-t pt-3 text-sm sm:grid-cols-3">
      <div>
        <span className="text-muted-foreground block text-xs">{t("artifactManifests")}</span>
        <strong>{report.applied ? report.deleted_manifest_ids.length : report.candidate_manifest_ids.length}</strong>
      </div>
      <div>
        <span className="text-muted-foreground block text-xs">{t("artifactFiles")}</span>
        <strong>{report.applied ? report.deleted_paths.length : report.candidate_orphan_paths.length}</strong>
      </div>
      <div>
        <span className="text-muted-foreground block text-xs">{t("artifactFailures")}</span>
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
  const { t } = useI18n();
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
            toast.success(t("artifactCleanupCompleted"), {
              description: t("artifactCleanupRemoved", { manifests: report.deleted_manifest_ids.length, files: report.deleted_paths.length }),
            });
          }
        },
        onError: (error) => toast.error(apply ? t("artifactCleanupFailed") : t("artifactCleanupPlanFailed"), { description: error.message }),
      },
    );
  }

  return (
    <section className="flex flex-col gap-3 border-b pb-4">
      <SectionHeading
        title={t("artifactRetentionTitle")}
        description={t("artifactRetentionDescription")}
      />
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex min-w-36 flex-1 flex-col gap-1 text-xs text-muted-foreground">
          {t("artifactRetentionHours")}
          <Input
            inputMode="numeric"
            min={1}
            value={retentionHours}
            onChange={(event) => setRetentionHours(event.target.value)}
          />
        </label>
        <Button type="button" variant="outline" disabled={!valid || cleanup.isPending} onClick={() => run(false)}>
          {cleanup.isPending ? <Spinner data-icon="inline-start" /> : <SearchIcon data-icon="inline-start" />}
          {t("artifactPreview")}
        </Button>
        <Button type="button" variant="destructive" disabled={!valid || cleanup.isPending} onClick={() => setConfirmOpen(true)}>
          <Trash2Icon data-icon="inline-start" />
          {t("apply")}
        </Button>
      </div>
      {cleanup.data ? <CleanupReportView report={cleanup.data} /> : null}
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia><ShieldAlertIcon /></AlertDialogMedia>
            <AlertDialogTitle>{t("artifactApplyCleanupTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("artifactApplyCleanupDescription", { hours })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={cleanup.isPending}>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={!valid || cleanup.isPending}
              onClick={(event) => {
                event.preventDefault();
                run(true);
              }}
            >
              {cleanup.isPending ? <Spinner data-icon="inline-start" /> : <Trash2Icon data-icon="inline-start" />}
              {t("artifactApplyCleanup")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

function OrphanRows({ files }: { files: OrphanArtifactFile[] }) {
  const { t } = useI18n();
  if (!files.length) return null;
  return (
    <section className="flex flex-col gap-2 border-t pt-3">
      <SectionHeading title={t("artifactUnregisteredFiles")} badge={files.length} />
      {files.map((file) => (
        <EntityRow
          key={file.path}
          variant="inline"
          actions={<Badge variant={file.managed_layout ? "outline" : "destructive"}>{file.managed_layout ? t("artifactManagedLayout") : t("artifactMalformedLayout")}</Badge>}
        >
          <div className="flex min-w-0 flex-col gap-1">
            <PathText value={file.path} tone="default" wrap="all" />
            <span className="text-xs text-muted-foreground">{formatBytes(file.byte_size)} · {t("artifactModified")} {formatDateTime(file.modified_at_ms)}</span>
          </div>
        </EntityRow>
      ))}
    </section>
  );
}

function ArtifactsView({ exportsOnly = false }: { exportsOnly?: boolean }) {
  const { t } = useI18n();
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
  if (inspection.error) return <PageError title={t("artifactInspectionFailed")} message={inspection.error.message} />;
  if (!inspection.data) return <PageEmpty title={t("artifactNoInspectionData")} description={t("artifactInspectionNoReport")} />;

  const selected = rows.find((entry) => entry.manifest.id === selectedId) ?? rows[0] ?? null;
  return (
    <div className="grid gap-4 xl:min-h-0 xl:flex-1 xl:grid-cols-[minmax(360px,0.85fr)_minmax(0,1.15fr)]">
      <section className="flex min-w-0 flex-col gap-3 border-r-0 xl:min-h-0 xl:border-r xl:pr-4">
        {!exportsOnly ? <CleanupControls /> : null}
        <div className="grid gap-2 sm:grid-cols-2">
          {!exportsOnly ? (
            <Select value={kind} onValueChange={(value) => setKind(value as ArtifactManifestKind | "all")}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>{artifactKinds.map((value) => <SelectItem key={value} value={value}>{artifactKindLabel(value, t)}</SelectItem>)}</SelectContent>
            </Select>
          ) : null}
          <Select value={verification} onValueChange={(value) => setVerification(value as ArtifactVerificationStatus | "all")}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>{verificationStates.map((value) => <SelectItem key={value} value={value}>{verificationLabel(value, t)}</SelectItem>)}</SelectContent>
          </Select>
          {!exportsOnly ? (
            <Select value={retention} onValueChange={(value) => setRetention(value as ArtifactRetentionState | "all")}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>{retentionStates.map((value) => <SelectItem key={value} value={value}>{retentionLabel(value, t)}</SelectItem>)}</SelectContent>
            </Select>
          ) : null}
          <div className={cn("relative", exportsOnly && "sm:col-span-1")}>
            <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input className="pl-9" placeholder={t("artifactSearchPlaceholder")} value={search} onChange={(event) => setSearch(event.target.value)} />
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
            )) : <PageEmpty title={exportsOnly ? t("artifactNoExports") : t("artifactNoMatching")} description={t("artifactAdjustFilters")} />}
            {!exportsOnly ? <OrphanRows files={inspection.data.orphan_files} /> : null}
          </div>
        </ScrollArea>
      </section>
      <section className={cn("min-w-0 border-t pt-4 xl:min-h-0 xl:border-t-0 xl:pt-0", !selected && "hidden xl:block")}>
        {selected ? <ManifestDetail entry={selected} /> : (
          <PageEmpty title={exportsOnly ? t("artifactNoExportSelected") : t("artifactNoArtifactSelected")} description={t("artifactSelectDescription")} />
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
  const { t } = useI18n();
  const backup = view.entry.backup;
  const latest = view.entry.latest_restore;
  const sessionHref = backup.provider_id && backup.provider_session_id
    ? `/sessions/${encodeURIComponent(backup.provider_id)}/${encodeURIComponent(backup.provider_session_id)}`
    : null;

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <SectionHeading
        title={backup.id}
        description={backup.provider_id || t("artifactNoProviderIdentity")}
        actions={(
          <>
            <Badge variant={statusVariant(view.verification.status)}>{verificationLabel(view.verification.status, t)}</Badge>
            <Button type="button" disabled={view.verification.status !== "verified"} onClick={() => onRestore(view)}>
              <ArchiveRestoreIcon data-icon="inline-start" />
              {t("artifactRestore")}
            </Button>
          </>
        )}
      />
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-3">
          <DetailLine label={t("artifactArtifactPath")}><PathText value={backup.artifact.path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label={t("artifactSourcePath")}><PathText value={backup.source_path} tone="default" wrap="all" /></DetailLine>
          <DetailLine label={t("provider")}>{backup.provider_id || "-"}</DetailLine>
          <DetailLine label={t("artifactProviderSession")}>
            {sessionHref ? <Link className="break-all underline underline-offset-4" to={sessionHref}>{backup.provider_session_id}</Link> : backup.provider_session_id || "-"}
          </DetailLine>
          <DetailLine label={t("artifactCanonicalSession")}>{backup.session_id || "-"}</DetailLine>
          <DetailLine label={t("artifactOperation")}>{backup.operation_id || "-"}</DetailLine>
          <DetailLine label={t("artifactCreated")}>{formatDateTime(backup.created_at_ms)}</DetailLine>
          <DetailLine label={t("size")}>{formatBytes(backup.artifact.byte_size)}</DetailLine>
          <DetailLine label={t("artifactFormat")}>{backup.artifact.format || "-"}</DetailLine>
          <DetailLine label={t("artifactHash")}><span className="break-all font-mono text-xs">{backup.artifact.content_hash}</span></DetailLine>
          <DetailLine label={t("artifactRestoreStatus")}>
            {latest ? <Badge variant={restoreVariant(latest.status)}>{t(latest.status === "success" ? "artifactRestoreStateSuccess" : latest.status === "failed" ? "artifactRestoreStateFailed" : "artifactRestoreStateRunning")}</Badge> : t("artifactNeverRestored")}
          </DetailLine>
          <DetailLine label={t("artifactRestoreActor")}>{latest?.actor || "-"}</DetailLine>
          <DetailLine label={t("artifactRestoreStarted")}>{latest ? formatDateTime(latest.started_at_ms) : "-"}</DetailLine>
          <DetailLine label={t("artifactRestoreFinished")}>{latest?.finished_at_ms ? formatDateTime(latest.finished_at_ms) : "-"}</DetailLine>
          <DetailLine label={t("artifactRestoreError")}><span className={cn(latest?.error && "text-destructive")}>{latest?.error || "-"}</span></DetailLine>
          <DetailLine label={t("artifactRestoreHint")}>{backup.restore_hint || "-"}</DetailLine>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">{t("artifactRestoreMetadata")}</span>
            <MetadataBlock value={backup.metadata} />
          </div>
          <div className="flex flex-col gap-2">
            <span className="text-muted-foreground font-mono text-xs uppercase">{t("artifactArtifactMetadata")}</span>
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
  const { t } = useI18n();
  const restore = useRestoreBackup();
  const backup = target?.entry.backup;

  function applyRestore() {
    if (!backup) return;
    restore.mutate(backup.id, {
      onSuccess: (record) => {
        onOpenChange(false);
        toast.success(t("artifactNativeBackupRestored"), { description: `${backup.provider_id || t("provider")} · ${record.id}` });
      },
      onError: (error) => toast.error(t("artifactBackupRestoreFailed"), { description: error.message }),
    });
  }

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogMedia><DatabaseBackupIcon /></AlertDialogMedia>
          <AlertDialogTitle>{t("artifactRestoreTitle")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("artifactRestoreDescription", { provider: backup?.provider_id || t("provider"), session: backup?.provider_session_id || "-" })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="rounded-md border p-3">
          <PathText value={backup?.artifact.path} tone="default" wrap="all" />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={restore.isPending}>{t("cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={!backup || restore.isPending}
            onClick={(event) => {
              event.preventDefault();
              applyRestore();
            }}
          >
            {restore.isPending ? <Spinner data-icon="inline-start" /> : <ArchiveRestoreIcon data-icon="inline-start" />}
            {t("artifactRestore")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function BackupsView() {
  const { t } = useI18n();
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
  if (backups.error) return <PageError title={t("artifactBackupsLoadFailed")} message={backups.error.message} />;

  return (
    <>
      <div className="grid gap-4 xl:min-h-0 xl:flex-1 xl:grid-cols-[minmax(360px,0.85fr)_minmax(0,1.15fr)]">
        <section className="flex min-w-0 flex-col gap-3 border-r-0 xl:min-h-0 xl:border-r xl:pr-4">
          <div className="grid gap-2 sm:grid-cols-2">
            <Input placeholder={t("artifactProviderPlaceholder")} value={provider} onChange={(event) => setProvider(event.target.value)} />
            <Input placeholder={t("artifactProviderSessionPlaceholder")} value={providerSessionId} onChange={(event) => setProviderSessionId(event.target.value)} />
            <Select value={restoreStatus} onValueChange={(value) => setRestoreStatus(value as BackupRestoreStatus | "all")}>
              <SelectTrigger className="sm:col-span-2"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("artifactAllRestoreStates")}</SelectItem>
                <SelectItem value="success">{t("artifactRestoreStateSuccess")}</SelectItem>
                <SelectItem value="failed">{t("artifactRestoreStateFailed")}</SelectItem>
                <SelectItem value="running">{t("artifactRestoreStateRunning")}</SelectItem>
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
                    actions={<Badge variant={statusVariant(view.verification.status)}>{verificationLabel(view.verification.status, t)}</Badge>}
                  >
                    <div className="flex min-w-0 flex-col gap-1">
                      <strong className="truncate text-sm font-medium">{backup.provider_session_id || backup.id}</strong>
                      <span className="truncate font-mono text-xs text-muted-foreground">{backup.artifact.path}</span>
                      <span className="text-xs text-muted-foreground">
                        {backup.provider_id || "-"} · {formatBytes(backup.artifact.byte_size)} · {latest ? `${t("artifactRestore")} ${t(latest.status === "success" ? "artifactRestoreStateSuccess" : latest.status === "failed" ? "artifactRestoreStateFailed" : "artifactRestoreStateRunning")}` : t("artifactNeverRestored")}
                      </span>
                    </div>
                  </EntityRow>
                );
              }) : <PageEmpty title={t("artifactNoBackups")} description={t("artifactNoBackupsDescription")} />}
            </div>
          </ScrollArea>
        </section>
        <section className={cn("min-w-0 border-t pt-4 xl:min-h-0 xl:border-t-0 xl:pt-0", !selected && "hidden xl:block")}>
          {detail.isLoading ? <PageSkeleton /> : detail.error ? (
            <PageError title={t("artifactBackupDetailLoadFailed")} message={detail.error.message} />
          ) : selected ? (
            <BackupDetail view={selected} onRestore={setRestoreTarget} />
          ) : <PageEmpty title={t("artifactNoBackupSelected")} description={t("artifactBackupSelectDescription")} />}
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
  const { t } = useI18n();
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
        title={t("artifactRegistry")}
        description={t("artifactRegistryDescription")}
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
            {t("artifactRefresh")}
          </Button>
        )}
      />
      <MetricGrid columns="auto" className="grid-cols-2">
        <MetricTile label={t("artifactRegistered")} value={entries.length} hint={t("artifactManifestsHint")} variant="compact" />
        <MetricTile label={t("artifactVerified")} value={verified} hint={t("artifactVerifiedHint")} variant="compact" />
        <MetricTile label={t("artifactAttention")} value={attention + (inspection.data?.orphan_files.length ?? 0)} hint={t("artifactAttentionHint")} variant="compact" />
        <MetricTile label={t("artifactBackupsExports")} value={`${backups.data?.length ?? 0} / ${exports}`} hint={t("artifactRecordsHint")} variant="compact" />
      </MetricGrid>
      <Tabs value={view} onValueChange={setView} className="min-w-0 xl:min-h-0 xl:flex-1">
        <TabsList>
          <TabsTrigger value="artifacts"><FileCheck2Icon />{t("artifactArtifactsTab")}</TabsTrigger>
          <TabsTrigger value="backups"><DatabaseBackupIcon />{t("artifactBackupsTab")}</TabsTrigger>
          <TabsTrigger value="exports"><FileOutputIcon />{t("artifactExportsTab")}</TabsTrigger>
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
