import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeftIcon, ArchiveRestoreIcon, BoxIcon, FileArchiveIcon } from "lucide-react";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
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
import { useManagerMeta, useManagerPreview } from "@/features/manager/queries";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { useCompressionArchive, useCompressionArchives, useCompressionProviders, useRestoreCompressionArchive } from "@/features/compression/queries";
import { SessionBlock } from "@/features/sessions/session-block";
import { compactPath, formatBytes, formatDateTime } from "@/lib/format";
import type { CompressionArchive, CompressionArchiveSummary, CompressionFormat, CompressionProviderSupport, ManagerItem, SessionEvent } from "@/lib/types";

type RestoreTarget = {
  archiveRef: string;
  title: string;
};

function formatRatio(value: number | null | undefined) {
  if (value === null || value === undefined || Number.isNaN(value)) return "-";
  return `${(value * 100).toFixed(value < 0.1 ? 1 : 0)}%`;
}

function archiveTitle(archive: CompressionArchiveSummary | CompressionArchive, fallback: string) {
  return archive.canonical_id || fallback || "Compression Archive";
}

function defaultRestorePrefix(archiveRef: string) {
  const prefix = archiveRef
    .replace(/^memorph-archive:\/\//, "")
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return prefix || "compression_archive";
}

function WorkspaceSummary({ workspace }: { workspace: string | null | undefined }) {
  return (
    <section className="flex flex-col gap-2 border-b pb-4" data-compression-workspace-summary>
      <span className="font-mono text-xs uppercase text-muted-foreground">Workspace</span>
      <strong className="break-words text-lg font-semibold">{workspace ? compactPath(workspace) : "Current workspace"}</strong>
      <span className="break-words font-mono text-xs text-muted-foreground">{workspace || "-"}</span>
    </section>
  );
}

function ProviderSupportList({ providers }: { providers: CompressionProviderSupport[] }) {
  if (!providers.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No providers</EmptyTitle>
          <EmptyDescription>No compression provider support was reported.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <section className="flex min-h-0 flex-col gap-3" data-compression-provider-support>
      <div className="flex items-center justify-between gap-3">
        <strong className="text-sm font-medium">Providers</strong>
        <Badge variant="outline">{providers.length}</Badge>
      </div>
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-2">
          {providers.map((provider) => (
            <div key={provider.provider_id} className="rounded-md border px-3 py-2">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium">{provider.provider_id}</div>
                  <div className="mt-1 flex flex-wrap gap-2">
                    <Badge variant={provider.detects_native_source ? "secondary" : "outline"}>
                      {provider.detects_native_source ? "Detects native" : "Portable only"}
                    </Badge>
                    <Badge variant={provider.native_target_projection ? "secondary" : "outline"}>
                      {provider.native_target_projection ? "Native target" : "Portable target"}
                    </Badge>
                  </div>
                </div>
                <Badge variant="outline">{provider.default_projection}</Badge>
              </div>
            </div>
          ))}
        </div>
      </ScrollArea>
    </section>
  );
}

function CompressionControlPanel({
  workspace,
  providers,
}: {
  workspace: string | null | undefined;
  providers: CompressionProviderSupport[];
}) {
  return (
    <Card className="min-h-0" data-manager-control-panel>
      <CardContent className="flex h-full min-h-0 flex-col gap-4">
        <WorkspaceSummary workspace={workspace} />
        <ProviderSupportList providers={providers} />
      </CardContent>
    </Card>
  );
}

function SectionHeader({ title, count }: { title: string; count: number }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b pb-2" data-compression-section-head>
      <strong className="text-sm font-medium">{title}</strong>
      <Badge variant="outline">{count}</Badge>
    </div>
  );
}

function CandidateRow({ item, onCompress }: { item: ManagerItem; onCompress: (item: ManagerItem) => void }) {
  const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
  return (
    <article className="rounded-md border px-3 py-3" data-compression-candidate-row>
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
        <div className="flex min-w-0 flex-col gap-2">
          <Link to={href} className="truncate text-sm font-medium hover:underline">
            {item.title || item.session_id}
          </Link>
          <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
            <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
            <span>{formatBytes(item.size_bytes)}</span>
            <span>Updated {formatDateTime(item.last_active_at)}</span>
          </div>
          <span className="truncate font-mono text-xs text-muted-foreground">{compactPath(item.project_dir || item.source_path)}</span>
        </div>
        <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-compression-row-actions>
          <Button asChild variant="outline" size="sm">
            <Link to={href}>View</Link>
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => onCompress(item)}>
            Compression
          </Button>
        </div>
      </div>
    </article>
  );
}

function ArchiveRow({ archive, onRestore }: { archive: CompressionArchiveSummary; onRestore: (archive: CompressionArchiveSummary) => void }) {
  const archiveRef = archive.archive_ref || "";
  const href = `/compression?archive_ref=${encodeURIComponent(archiveRef)}`;
  return (
    <article className="rounded-md border px-3 py-3" data-compression-archive-row>
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
        <div className="flex min-w-0 flex-col gap-2">
          <Link to={href} className="truncate text-sm font-medium hover:underline">
            {archive.canonical_id || "Compression Archive"}
          </Link>
          <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span>{archive.source_provider_id || "-"} -&gt; {archive.target_provider_id || "-"}</span>
            <span>sourceEvents={archive.source_event_count ?? 0}</span>
            <span>storedSize={formatBytes(archive.stored_size_bytes)}</span>
            <span>originalSize={formatBytes(archive.original_size_bytes)}</span>
            <span>compressionRatio={formatRatio(archive.compression_ratio)}</span>
            <span>createdAt={formatDateTime(archive.created_at)}</span>
          </div>
          <span className="break-all font-mono text-xs text-muted-foreground">{archiveRef}</span>
          {archive.workspace_dir ? <span className="truncate font-mono text-xs text-muted-foreground">{compactPath(archive.workspace_dir)}</span> : null}
        </div>
        <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-compression-row-actions>
          <Button asChild variant="outline" size="sm">
            <Link to={href}>View</Link>
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => onRestore(archive)}>
            Restore
          </Button>
        </div>
      </div>
    </article>
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
          toast.success("Restore", { description: result.files?.length ? result.files.join(", ") : "Archive restored" });
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg" data-restore-compression-dialog>
        <DialogHeader>
          <DialogTitle>Restore Compression Archive</DialogTitle>
          <DialogDescription>{target ? target.title : "Choose a compression archive to restore."}</DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <input type="hidden" name="archive_ref" value={target?.archiveRef || ""} />
            <FieldLabel>Archive Ref</FieldLabel>
            <div className="break-all rounded-md border p-3 font-mono text-xs" data-compression-restore-archive-ref>
              {target?.archiveRef || "-"}
            </div>
          </Field>
          <Field>
            <FieldLabel htmlFor="restore-output-prefix">Output Prefix</FieldLabel>
            <Input
              id="restore-output-prefix"
              value={currentDraft.outputPrefix}
              onChange={(event) => setDraft({ key: targetKey, outputPrefix: event.target.value, format: currentDraft.format })}
            />
          </Field>
          <Field>
            <FieldLabel>Format</FieldLabel>
            <Select
              value={currentDraft.format}
              onValueChange={(value) => setDraft({ key: targetKey, outputPrefix: currentDraft.outputPrefix, format: value as CompressionFormat })}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Format" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {(["json", "md", "html", "morph", "both"] as CompressionFormat[]).map((item) => (
                    <SelectItem key={item} value={item}>{item}</SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
            <FieldDescription>Restored files are written by the backend using this prefix and format.</FieldDescription>
          </Field>
          {files.length ? (
            <Field>
              <FieldLabel>Files</FieldLabel>
              <div className="flex max-h-40 flex-col gap-2 overflow-auto rounded-md border p-3">
                {files.map((file) => <span key={file} className="break-all font-mono text-xs">{file}</span>)}
              </div>
            </Field>
          ) : null}
        </FieldGroup>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => handleOpenChange(false)} disabled={restoreMutation.isPending}>Cancel</Button>
          <Button type="button" onClick={restoreArchive} disabled={!target || restoreMutation.isPending}>
            {restoreMutation.isPending ? <Spinner data-icon="inline-start" /> : <ArchiveRestoreIcon data-icon="inline-start" />}
            Restore
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CompressionOverview() {
  const [compressTarget, setCompressTarget] = useState<ManagerItem | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<RestoreTarget | null>(null);
  const meta = useManagerMeta();
  const archives = useCompressionArchives({ limit: 100 });
  const providers = useCompressionProviders();
  const candidates = useManagerPreview({ sort: "size", limit: 50 });

  if (archives.isLoading || providers.isLoading || candidates.isLoading || meta.isLoading) return <PageSkeleton />;
  if (archives.error) return <PageError title="Compression archives failed to load" message={archives.error.message} />;
  if (providers.error) return <PageError title="Compression providers failed to load" message={providers.error.message} />;
  if (candidates.error) return <PageError title="Compression candidates failed to load" message={candidates.error.message} />;
  if (meta.error) return <PageError title="Compression workspace failed to load" message={meta.error.message} />;

  const archiveRows = archives.data ?? [];
  const providerRows = providers.data ?? [];
  const candidateRows = candidates.data?.items ?? [];

  return (
    <div className="grid min-h-[calc(100vh-124px)] grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,0.44fr)_minmax(0,1fr)]" data-manager-page-layout>
      <CompressionControlPanel workspace={meta.data?.selected_workspace} providers={providerRows} />
      <Card className="min-h-0" data-manager-result-panel>
        <CardContent className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)_auto_minmax(0,1fr)] gap-3">
          <SectionHeader title="Compress Sessions" count={candidates.data?.total_count ?? candidateRows.length} />
          <ScrollArea className="min-h-0 pr-3">
            <div className="flex flex-col gap-2">
              {candidateRows.length ? (
                candidateRows.map((item) => <CandidateRow key={`${item.provider_id}:${item.session_id}`} item={item} onCompress={setCompressTarget} />)
              ) : (
                <PageEmpty title="No sessions" description="No recent sessions are available for compression." />
              )}
            </div>
          </ScrollArea>
          <SectionHeader title="Compression Archives" count={archiveRows.length} />
          <ScrollArea className="min-h-0 pr-3">
            <div className="flex flex-col gap-2">
              {archiveRows.length ? (
                archiveRows.map((archive) => (
                  <ArchiveRow
                    key={archive.archive_ref}
                    archive={archive}
                    onRestore={(item) => setRestoreTarget({ archiveRef: item.archive_ref, title: item.canonical_id || "Compression Archive" })}
                  />
                ))
              ) : (
                <PageEmpty title="No archives" description="No compression archives have been created yet." />
              )}
            </div>
          </ScrollArea>
        </CardContent>
      </Card>
      <CompressSessionDialog
        open={Boolean(compressTarget)}
        target={compressTarget ? {
          providerId: compressTarget.provider_id,
          sessionId: compressTarget.session_id,
          title: compressTarget.title || compressTarget.session_id,
          workspace: compressTarget.project_dir,
        } : null}
        onOpenChange={(open) => {
          if (!open) setCompressTarget(null);
        }}
      />
      <RestoreCompressionDialog
        open={Boolean(restoreTarget)}
        target={restoreTarget}
        onOpenChange={(open) => {
          if (!open) setRestoreTarget(null);
        }}
      />
    </div>
  );
}

function MetadataRow({ label, value }: { label: string; value: string | number | null | undefined }) {
  return (
    <div className="grid gap-1 md:grid-cols-[10rem_1fr]">
      <span className="text-muted-foreground">{label}</span>
      <span className="break-words font-medium">{value === null || value === undefined || value === "" ? "-" : value}</span>
    </div>
  );
}

function EventSection({ event, index }: { event: SessionEvent; index: number }) {
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
              <EmptyTitle>No blocks</EmptyTitle>
              <EmptyDescription>This archived event has metadata but no rendered content blocks.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          event.blocks.map((block, blockIndex) => <SessionBlock key={`${event.id}-${blockIndex}`} block={block} />)
        )}
      </div>
    </section>
  );
}

function CompressionArchiveDetail({ archiveRef }: { archiveRef: string }) {
  const [restoreOpen, setRestoreOpen] = useState(false);
  const archive = useCompressionArchive(archiveRef);

  if (archive.isLoading) return <PageSkeleton />;
  if (archive.error) return <PageError title="Archive failed to load" message={archive.error.message} />;
  if (!archive.data) return <PageEmpty title="Archive not found" description="Return to compression archives and choose another archive." />;

  const data = archive.data;
  const title = archiveTitle(data, archiveRef);
  const sourceEventCount = data.source_event_ids?.length ?? data.events.length;

  return (
    <div className="flex min-h-[calc(100vh-124px)] flex-col gap-4">
      <section className="grid gap-3 border-b pb-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-start" data-session-header>
        <div className="min-w-0">
          <p className="font-mono text-xs uppercase text-muted-foreground">Compression Archive Detail</p>
          <h1 className="break-words text-2xl font-semibold">{title}</h1>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span>archiveRef=<code>{archiveRef}</code></span>
            <span>{data.source_provider_id || "-"} -&gt; {data.target_provider_id || "-"}</span>
            <span>sourceEvents={sourceEventCount}</span>
            <span>createdAt={formatDateTime(data.created_at)}</span>
            {data.workspace_dir ? <span>workspace=<code>{data.workspace_dir}</code></span> : null}
          </div>
        </div>
        <div className="flex flex-wrap justify-start gap-2 md:justify-end">
          <Button asChild variant="outline">
            <Link to="/compression">
              <ArrowLeftIcon data-icon="inline-start" />
              Back
            </Link>
          </Button>
          <Button type="button" variant="outline" onClick={() => setRestoreOpen(true)}>
            <ArchiveRestoreIcon data-icon="inline-start" />
            Restore
          </Button>
        </div>
      </section>
      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[4rem_minmax(0,1fr)]" data-detail-layout data-compression-detail-layout>
        <aside className="hidden min-h-0 rounded-md border p-2 lg:flex lg:flex-col lg:gap-1" aria-label="Timeline" data-detail-timeline>
          {data.events.length ? data.events.map((event) => <span key={event.id} className="min-h-3 flex-1 rounded-sm bg-muted" />) : null}
        </aside>
        <section className="grid min-h-0 grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(280px,0.38fr)]">
          <Card className="min-h-0">
            <CardContent className="h-full min-h-0">
              <ScrollArea className="h-full pr-3">
                <div className="flex flex-col gap-5">
                  {data.events.length ? data.events.map((event, index) => <EventSection key={event.id} event={event} index={index} />) : (
                    <PageEmpty title="No archived events" description="This archive has metadata but no embedded events." />
                  )}
                </div>
              </ScrollArea>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="flex flex-col gap-3">
              <div className="flex items-center gap-2">
                <FileArchiveIcon aria-hidden="true" />
                <strong>Archive Metadata</strong>
              </div>
              <MetadataRow label="Archive Ref" value={archiveRef} />
              <MetadataRow label="Canonical ID" value={data.canonical_id} />
              <MetadataRow label="Source Provider" value={data.source_provider_id} />
              <MetadataRow label="Target Provider" value={data.target_provider_id} />
              <MetadataRow label="Workspace" value={data.workspace_dir} />
              <MetadataRow label="Summary Event" value={data.summary_event_id} />
              <MetadataRow label="Created" value={formatDateTime(data.created_at)} />
              <Separator />
              <div className="flex flex-col gap-2">
                <div className="flex items-center gap-2 text-sm font-medium">
                  <BoxIcon aria-hidden="true" />
                  Source Events
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

  return archiveRef ? <CompressionArchiveDetail archiveRef={archiveRef} /> : <CompressionOverview />;
}
