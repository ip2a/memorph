import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeftIcon, ArchiveRestoreIcon, BoxIcon, FileArchiveIcon } from "lucide-react";
import { DetailHeader } from "@/components/shared/detail-header";
import { EntityRow } from "@/components/shared/entity-row";
import { MetaLine } from "@/components/shared/meta-line";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { SectionHeading } from "@/components/shared/section-heading";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useManagerMeta } from "@/features/manager/queries";
import { useCompressionArchive, useCompressionArchives, useRestoreCompressionArchive } from "@/features/compression/queries";
import { SessionBlock } from "@/features/sessions/session-block";
import { formatBytes, formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { CompressionArchive, CompressionArchiveSummary, CompressionFormat, SessionEvent } from "@/lib/types";

type RestoreTarget = {
  archiveRef: string;
  title: string;
};

type CompressedSessionEntry = {
  key: string;
  providerId: string;
  sessionId: string;
  archives: CompressionArchiveSummary[];
  latestCreatedAt: string;
  totalStoredBytes: number;
  workspaceDir?: string | null;
};

function sessionKey(providerId: string, sessionId: string) {
  return `${providerId}:${sessionId}`;
}

function groupArchivesBySession(archives: CompressionArchiveSummary[]): CompressedSessionEntry[] {
  const map = new Map<string, CompressedSessionEntry>();
  for (const archive of archives) {
    const key = sessionKey(archive.source_provider_id, archive.canonical_id);
    const existing = map.get(key);
    if (existing) {
      existing.archives.push(archive);
      existing.totalStoredBytes += archive.stored_size_bytes;
      if (archive.created_at > existing.latestCreatedAt) {
        existing.latestCreatedAt = archive.created_at;
      }
      if (!existing.workspaceDir && archive.workspace_dir) {
        existing.workspaceDir = archive.workspace_dir;
      }
    } else {
      map.set(key, {
        key,
        providerId: archive.source_provider_id,
        sessionId: archive.canonical_id,
        archives: [archive],
        latestCreatedAt: archive.created_at,
        totalStoredBytes: archive.stored_size_bytes,
        workspaceDir: archive.workspace_dir,
      });
    }
  }
  return [...map.values()]
    .map((entry) => ({
      ...entry,
      archives: [...entry.archives].sort((left, right) => right.created_at.localeCompare(left.created_at)),
    }))
    .sort((left, right) => right.latestCreatedAt.localeCompare(left.latestCreatedAt));
}

function archiveTitle(archive: CompressionArchive, fallback: string, archiveLabel: string) {
  return archive.canonical_id || fallback || archiveLabel;
}

function defaultRestorePrefix(archiveRef: string) {
  const prefix = archiveRef
    .replace(/^memorph-archive:\/\//, "")
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return prefix || "compression_archive";
}

function CompressedSessionRow({
  entry,
  selected,
  onSelect,
}: {
  entry: CompressedSessionEntry;
  selected: boolean;
  onSelect: (key: string) => void;
}) {
  const { t } = useI18n();
  return (
    <SelectableRowButton
      data-compression-session-row
      selected={selected}
      title={entry.sessionId}
      leading={<ProviderLogo providerId={entry.providerId} size="sm" alt={entry.providerId} />}
      meta={(
        <span className="flex flex-col gap-0.5">
          <span className="flex flex-wrap items-center gap-2">
            <span>{entry.providerId}</span>
            <span>{t("compressionArchiveCount", { count: entry.archives.length })}</span>
            <span>{formatBytes(entry.totalStoredBytes)}</span>
          </span>
          {entry.workspaceDir ? (
            <span className="truncate font-mono text-[11px]" title={entry.workspaceDir}>
              {entry.workspaceDir}
            </span>
          ) : null}
        </span>
      )}
      onClick={() => onSelect(entry.key)}
    />
  );
}

function ArchiveSummaryRow({ archive, onRestore }: { archive: CompressionArchiveSummary; onRestore: (target: RestoreTarget) => void }) {
  const { t } = useI18n();
  const href = `/compression?session=${encodeURIComponent(sessionKey(archive.source_provider_id, archive.canonical_id))}&archive_ref=${encodeURIComponent(archive.archive_ref)}`;
  const title = archive.canonical_id || archive.archive_ref;
  return (
    <EntityRow
      data-compression-archive-row
      actionsProps={{ "data-compression-row-actions": true }}
      actions={(
        <>
          <Button asChild variant="outline">
            <Link to={href}>{t("view")}</Link>
          </Button>
          <Button type="button" variant="outline" onClick={() => onRestore({ archiveRef: archive.archive_ref, title })}>
            {t("resume")}
          </Button>
        </>
      )}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <Link to={href} className="truncate text-sm font-medium hover:underline">
          {title}
        </Link>
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            {archive.source_provider_id ? (
              <ProviderLogo providerId={archive.source_provider_id} size="xs" alt={archive.source_provider_id} />
            ) : null}
            <span>{archive.source_provider_id || "-"}</span>
            <span aria-hidden="true">-&gt;</span>
            {archive.target_provider_id ? (
              <ProviderLogo providerId={archive.target_provider_id} size="xs" alt={archive.target_provider_id} />
            ) : null}
            <span>{archive.target_provider_id || "-"}</span>
          </span>
          <span>{t("compressionEvents", { count: archive.source_event_count })}</span>
          <span>{formatBytes(archive.stored_size_bytes)}</span>
          <span>{t("compressionCreated", { date: formatDateTime(archive.created_at) })}</span>
        </div>
        <PathText value={archive.workspace_dir || archive.archive_ref} fallback="-" wrap="all" />
      </div>
    </EntityRow>
  );
}

function RestoreCompressionDialog({
  target,
  open,
  onOpenChange,
}: {
  target: RestoreTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const targetKey = target?.archiveRef || "";
  const defaultPrefix = useMemo(() => defaultRestorePrefix(targetKey), [targetKey]);
  const [draft, setDraft] = useState<{ key: string; outputPrefix: string; format: CompressionFormat } | null>(null);
  const [fileState, setFileState] = useState<{ key: string; files: string[] } | null>(null);
  const restoreMutation = useRestoreCompressionArchive();
  const currentDraft = draft?.key === targetKey ? draft : { key: targetKey, outputPrefix: defaultPrefix, format: "json" as CompressionFormat };
  const files = fileState?.key === targetKey ? fileState.files : [];

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen) {
      setDraft(null);
      setFileState(null);
    }
    onOpenChange(nextOpen);
  }

  function restoreArchive() {
    if (!target) throw new Error("Missing compression archive target");
    restoreMutation.mutate(
      {
        archive_ref: target.archiveRef,
        output_prefix: currentDraft.outputPrefix || null,
        format: currentDraft.format,
      },
      {
        onSuccess: async (result) => {
          setFileState({ key: targetKey, files: result.files ?? [] });
          await queryClient.invalidateQueries({ queryKey: ["compression"] });
          toast.success(t("resume"), { description: result.files?.length ? result.files.join(", ") : t("compressionArchiveRestored") });
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg" data-restore-compression-dialog>
        <DialogHeader>
          <DialogTitle>{t("compressionRestoreTitle")}</DialogTitle>
          <DialogDescription>{target ? target.title : t("compressionRestoreDescription")}</DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <input type="hidden" name="archive_ref" value={target?.archiveRef || ""} />
            <FieldLabel>{t("compressionArchiveRef")}</FieldLabel>
            <div className="break-all rounded-md border p-3 font-mono text-xs" data-compression-restore-archive-ref>
              <PathText value={target?.archiveRef} fallback="-" tone="default" wrap="all" />
            </div>
          </Field>
          <Field>
            <FieldLabel htmlFor="restore-output-prefix">{t("compressionOutputPrefix")}</FieldLabel>
            <Input
              id="restore-output-prefix"
              value={currentDraft.outputPrefix}
              onChange={(event) => setDraft({ key: targetKey, outputPrefix: event.target.value, format: currentDraft.format })}
            />
          </Field>
          <Field>
            <FieldLabel>{t("sessionFormat")}</FieldLabel>
            <Select
              value={currentDraft.format}
              onValueChange={(value) => setDraft({ key: targetKey, outputPrefix: currentDraft.outputPrefix, format: value as CompressionFormat })}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t("sessionFormat")} />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {(["json", "md", "html", "morph", "both"] as CompressionFormat[]).map((item) => (
                    <SelectItem key={item} value={item}>{item}</SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
            <FieldDescription>{t("compressionRestoreFormatDescription")}</FieldDescription>
          </Field>
          {files.length ? (
            <Field>
              <FieldLabel>{t("compressionFiles")}</FieldLabel>
              <div className="flex max-h-40 flex-col gap-2 overflow-auto rounded-md border p-3">
                {files.map((file) => <span key={file} className="break-all font-mono text-xs">{file}</span>)}
              </div>
            </Field>
          ) : null}
        </FieldGroup>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => handleOpenChange(false)} disabled={restoreMutation.isPending}>{t("cancel")}</Button>
          <Button type="button" onClick={restoreArchive} disabled={!target || restoreMutation.isPending}>
            {restoreMutation.isPending ? <Spinner data-icon="inline-start" /> : <ArchiveRestoreIcon data-icon="inline-start" />}
            {t("resume")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CompressionOverview() {
  const { t } = useI18n();
  const [searchParams, setSearchParams] = useSearchParams();
  const [restoreTarget, setRestoreTarget] = useState<RestoreTarget | null>(null);
  const meta = useManagerMeta();
  const workspace = meta.data?.selected_workspace ?? undefined;
  const archives = useCompressionArchives({ workspace, limit: 200 });
  const compressedSessions = useMemo(
    () => groupArchivesBySession(archives.data ?? []),
    [archives.data],
  );
  const selectedKey = searchParams.get("session") ?? "";
  const selectedSession = compressedSessions.find((entry) => entry.key === selectedKey) ?? null;

  useEffect(() => {
    if (!compressedSessions.length) return;
    if (selectedKey && compressedSessions.some((entry) => entry.key === selectedKey)) return;
    setSearchParams({ session: compressedSessions[0].key }, { replace: true });
  }, [compressedSessions, selectedKey, setSearchParams]);

  if (archives.isLoading || meta.isLoading) return <PageSkeleton />;
  if (archives.error) return <PageError title={t("compressionArchivesLoadFailed")} message={archives.error.message} />;
  if (meta.error) return <PageError title={t("compressionWorkspaceLoadFailed")} message={meta.error.message} />;

  const archiveRows = selectedSession?.archives ?? [];

  return (
    <div className="flex h-full min-h-0 flex-col">
    <TwoPanePage className="min-h-0 flex-1" data-compression-page-layout>
      <PanelCard className="flex min-h-0 flex-col gap-3 p-3" data-compression-session-panel>
        <SectionHeading title={t("compressionCompressedSessions")} badge={compressedSessions.length} />
        <ScrollArea className="min-h-0 flex-1 pr-3">
          <div className="flex flex-col gap-2">
            {compressedSessions.length ? (
              compressedSessions.map((entry) => (
                <CompressedSessionRow
                  key={entry.key}
                  entry={entry}
                  selected={entry.key === selectedKey}
                  onSelect={(key) => setSearchParams({ session: key })}
                />
              ))
            ) : (
              <PageEmpty title={t("compressionNoCompressedSessions")} description={t("compressionNoCompressedSessionsDescription")} />
            )}
          </div>
        </ScrollArea>
      </PanelCard>

      <PanelCard className="flex min-h-0 flex-col gap-3 p-3" data-compression-archive-panel>
        {selectedSession ? (
          <>
            <div className="flex flex-wrap items-start justify-between gap-3 border-b pb-3">
              <div className="flex min-w-0 flex-col gap-2">
                <SectionHeading title={t("compressionArchives")} badge={archiveRows.length} />
                <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
                  <ProviderLogo providerId={selectedSession.providerId} size="xs" alt={selectedSession.providerId} />
                  <span>{selectedSession.providerId}</span>
                  <span className="font-mono">{selectedSession.sessionId}</span>
                </div>
              </div>
              <Button asChild variant="outline">
                <Link to={`/sessions/${encodeURIComponent(selectedSession.providerId)}/${encodeURIComponent(selectedSession.sessionId)}`}>
                  {t("view")}
                </Link>
              </Button>
            </div>
            <ScrollArea className="min-h-0 flex-1 pr-3">
              <div className="flex flex-col gap-2">
                {archiveRows.length ? (
                  archiveRows.map((archive) => (
                    <ArchiveSummaryRow key={archive.archive_ref} archive={archive} onRestore={setRestoreTarget} />
                  ))
                ) : (
                  <PageEmpty title={t("compressionNoArchives")} description={t("compressionNoArchivesDescription")} />
                )}
              </div>
            </ScrollArea>
          </>
        ) : (
          <PageEmpty title={t("compressionSelectSession")} description={t("compressionSelectSessionDescription")} />
        )}
      </PanelCard>

      <RestoreCompressionDialog
        open={Boolean(restoreTarget)}
        target={restoreTarget}
        onOpenChange={(open) => {
          if (!open) setRestoreTarget(null);
        }}
      />
    </TwoPanePage>
    </div>
  );
}

function EventSection({ event, index }: { event: SessionEvent; index: number }) {
  const { t } = useI18n();
  return (
    <section className="flex flex-col gap-4">
      {index > 0 ? <Separator /> : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">{event.role}</Badge>
            <Badge variant="outline">{event.kind}</Badge>
            <Badge variant="outline">{event.metadata.fidelity}</Badge>
          </div>
          <div className="break-words text-sm font-medium">{event.id}</div>
          <div className="text-muted-foreground">{formatDateTime(event.timestamp)}</div>
        </div>
        {event.metadata.model ? <Badge variant="outline">{event.metadata.model}</Badge> : null}
      </div>
      <div className="flex flex-col gap-3">
        {event.blocks.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>{t("compressionNoBlocks")}</EmptyTitle>
              <EmptyDescription>{t("compressionNoBlocksDescription")}</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          event.blocks.map((block, blockIndex) => <SessionBlock key={`${event.id}-${blockIndex}`} block={block} />)
        )}
      </div>
    </section>
  );
}

function CompressionArchiveDetail({ archiveRef, sessionParam }: { archiveRef: string; sessionParam: string }) {
  const { t } = useI18n();
  const [restoreOpen, setRestoreOpen] = useState(false);
  const archive = useCompressionArchive(archiveRef);

  if (archive.isLoading) return <PageSkeleton />;
  if (archive.error) return <PageError title={t("compressionArchiveLoadFailed")} message={archive.error.message} />;
  if (!archive.data) return <PageEmpty title={t("compressionArchiveNotFound")} description={t("compressionArchiveNotFoundDescription")} />;

  const data = archive.data;
  const title = archiveTitle(data, archiveRef, t("compressionArchive"));
  const sourceEventCount = data.source_event_ids?.length ?? data.events.length;
  const sessionHref = sessionParam
    ? `/compression?session=${encodeURIComponent(sessionParam)}`
    : `/compression?session=${encodeURIComponent(sessionKey(data.source_provider_id, data.canonical_id))}`;

  return (
    <div className="flex min-h-[calc(100vh-124px)] flex-col gap-4">
      <DetailHeader
        data-session-header
        separated
        eyebrow={t("compressionArchiveDetail")}
        title={title}
        meta={(
          <>
            <span>{t("compressionArchiveRef")}: <code>{archiveRef}</code></span>
            <span>{data.source_provider_id || "-"} -&gt; {data.target_provider_id || "-"}</span>
            <span>{t("compressionSourceEvents", { count: sourceEventCount })}</span>
            <span>{t("compressionCreatedAt", { date: formatDateTime(data.created_at) })}</span>
            {data.workspace_dir ? <span>{t("workspace")}: <code>{data.workspace_dir}</code></span> : null}
          </>
        )}
        actions={(
          <>
          <Button asChild variant="outline">
            <Link to={sessionHref}>
              <ArrowLeftIcon data-icon="inline-start" />
              {t("back")}
            </Link>
          </Button>
          <Button type="button" variant="outline" onClick={() => setRestoreOpen(true)}>
            <ArchiveRestoreIcon data-icon="inline-start" />
            {t("resume")}
          </Button>
          </>
        )}
      />
      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[4rem_minmax(0,1fr)]" data-detail-layout data-compression-detail-layout>
        <aside className="hidden min-h-0 rounded-md border p-2 lg:flex lg:flex-col lg:gap-1" aria-label={t("compressionTimeline")} data-detail-timeline>
          {data.events.length ? data.events.map((event) => <span key={event.id} className="min-h-3 flex-1 rounded-sm bg-muted" />) : null}
        </aside>
        <section className="grid min-h-0 grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(280px,0.38fr)]">
          <Card className="min-h-0">
            <CardContent className="h-full min-h-0">
              <ScrollArea className="h-full pr-3">
                <div className="flex flex-col gap-5">
                  {data.events.length ? data.events.map((event, index) => <EventSection key={event.id} event={event} index={index} />) : (
                    <PageEmpty title={t("compressionNoArchivedEvents")} description={t("compressionNoArchivedEventsDescription")} />
                  )}
                </div>
              </ScrollArea>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="flex flex-col gap-3">
              <div className="flex items-center gap-2">
                <FileArchiveIcon aria-hidden="true" />
                <strong>{t("compressionArchiveMetadata")}</strong>
              </div>
              <MetaLine columns="wide" className="gap-1" label={t("compressionArchiveRef")} value={<PathText value={archiveRef} tone="default" weight="medium" wrap="all" className="text-sm" />} />
              <MetaLine columns="wide" className="gap-1" valueClassName="break-words font-medium" label={t("compressionCanonicalId")} value={data.canonical_id} />
              <MetaLine columns="wide" className="gap-1" valueClassName="break-words font-medium" label={t("compressionSourceProvider")} value={data.source_provider_id} />
              <MetaLine columns="wide" className="gap-1" valueClassName="break-words font-medium" label={t("compressionTargetProvider")} value={data.target_provider_id} />
              <MetaLine columns="wide" className="gap-1" label={t("workspace")} value={<PathText value={data.workspace_dir} tone="default" weight="medium" wrap="words" className="text-sm" />} />
              <MetaLine columns="wide" className="gap-1" valueClassName="break-words font-medium" label={t("compressionSummaryEvent")} value={data.summary_event_id} />
              <MetaLine columns="wide" className="gap-1" valueClassName="break-words font-medium" label={t("compressionCreatedLabel")} value={formatDateTime(data.created_at)} />
              <Separator />
              <div className="flex flex-col gap-2">
                <div className="flex items-center gap-2 text-sm font-medium">
                  <BoxIcon aria-hidden="true" />
                  {t("compressionSourceEventsLabel")}
                </div>
                <div className="flex max-h-56 flex-col gap-1 overflow-auto">
                  {data.source_event_ids.map((eventId) => <span key={eventId} className="break-all font-mono text-xs text-muted-foreground">{eventId}</span>)}
                </div>
              </div>
            </CardContent>
          </Card>
        </section>
      </div>
      <RestoreCompressionDialog
        open={restoreOpen}
        target={{ archiveRef, title }}
        onOpenChange={setRestoreOpen}
      />
    </div>
  );
}

export function CompressionPage() {
  const [searchParams] = useSearchParams();
  const archiveRef = searchParams.get("archive_ref") ?? "";
  const sessionParam = searchParams.get("session") ?? "";

  if (archiveRef) {
    return <CompressionArchiveDetail archiveRef={archiveRef} sessionParam={sessionParam} />;
  }
  return <CompressionOverview />;
}
