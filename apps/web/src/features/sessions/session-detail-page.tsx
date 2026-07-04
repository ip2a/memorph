import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArchiveIcon, PinIcon } from "lucide-react";
import { DetailHeader } from "@/components/shared/detail-header";
import { DetailTimeline, scrollToDetailMessage } from "@/components/shared/detail-timeline";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { formatDateTime, compactPath, formatDetailTitle } from "@/lib/format";
import type { SessionArtifact, SessionDetailView, SessionEvent } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSession } from "@/features/sessions/queries";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { getBlockLabel, SessionBlock } from "@/features/sessions/session-block";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/actions";
import { getMeta, listProviders } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

function detailTitle(view: SessionDetailView) {
  return view.display_title || view.title || view.native_title || view.session_id;
}

function MetaField({ name, value, compact = false }: { name: string; value: string; compact?: boolean }) {
  const display = compact ? compactPath(value) : value;
  return (
    <span title={`${name}=${value}`}>
      {name}=<code>{display}</code>
    </span>
  );
}

function getBlockLabels(blocks: SessionEvent["blocks"]) {
  return (blocks ?? []).map(getBlockLabel).filter(Boolean);
}

function SessionArtifactsDialog({
  artifacts,
  open,
  onOpenChange,
}: {
  artifacts: SessionArtifact[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-session-artifacts-dialog>
        <DialogHeader>
          <DialogTitle>Artifacts</DialogTitle>
          <DialogDescription>Files, images, patches, and attachments attached to this session.</DialogDescription>
        </DialogHeader>
        {artifacts.length === 0 ? (
          <p className="text-sm text-muted-foreground">No session artifacts were found for this canonical session.</p>
        ) : (
          <ScrollArea className="max-h-[min(60vh,32rem)] pr-3">
            <div className="flex flex-col gap-3">
              {artifacts.map((artifact) => (
                <div key={artifact.id} className="flex flex-col gap-1 border-b pb-3 last:border-b-0 last:pb-0">
                  <span className="break-all font-mono text-xs font-medium">{artifact.id}</span>
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <Badge variant="outline">{artifact.kind}</Badge>
                    <span>{artifact.mime_type ?? "-"}</span>
                  </div>
                  <PathText value={artifact.path} wrap="all" className="text-xs" />
                </div>
              ))}
            </div>
          </ScrollArea>
        )}
      </DialogContent>
    </Dialog>
  );
}

function DetailEventItem({ event, index, highlighted }: { event: SessionEvent; index: number; highlighted?: boolean }) {
  const role = (event.role ?? "unknown").replaceAll("_", " ");
  const kind = (event.kind ?? "unknown").replaceAll("_", " ");
  const blockLabels = getBlockLabels(event.blocks);
  const blocks = event.blocks ?? [];

  return (
    <article
      className={cn(
        "overflow-hidden border border-border",
        "data-[role=assistant]:border-l-[3px] data-[role=assistant]:border-l-[#e4e4de] data-[role=assistant]:bg-[#f4f4f1]",
        "data-[role=user]:border-l-[3px] data-[role=user]:border-l-[#d4dde6] data-[role=user]:bg-[#f0f4f8]",
        "data-[role=system]:border-l-[3px] data-[role=system]:border-l-border data-[role=system]:bg-muted/50",
        highlighted && "outline-2 outline-foreground/35 -outline-offset-2",
      )}
      data-message-index={index}
      data-role={event.role ?? "unknown"}
    >
      <header className="flex items-center justify-between gap-2 border-b px-2.5 py-2 font-mono text-xs">
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2 overflow-hidden">
          <span className="font-bold uppercase">{role}</span>
          <span>{kind}</span>
          {blockLabels.map((label) => (
            <span key={label} className="text-muted-foreground">{label}</span>
          ))}
          {event.metadata?.model ? <span className="text-muted-foreground">{event.metadata.model}</span> : null}
        </span>
        <span className="shrink-0 whitespace-nowrap text-muted-foreground">{formatDateTime(event.timestamp)}</span>
      </header>
      <div className="max-h-[min(52vh,560px)] overflow-auto p-3">
        {blocks.length === 0 ? (
          <p className="text-sm text-muted-foreground">No details.</p>
        ) : (
          <div className="flex flex-col gap-2.5">
            {blocks.map((block, blockIndex) => (
              <SessionBlock key={`${event.id}-${blockIndex}`} block={block} />
            ))}
          </div>
        )}
      </div>
    </article>
  );
}

export function SessionDetailPage() {
  const [renameOpen, setRenameOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [compressionOpen, setCompressionOpen] = useState(false);
  const [switchOpen, setSwitchOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [syncOpen, setSyncOpen] = useState(false);
  const [artifactsOpen, setArtifactsOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState<number | null>(null);
  const highlightTimeoutRef = useRef<number | null>(null);
  const { provider = "", sessionId = "" } = useParams();
  const session = useSession(provider, sessionId, { event_limit: 80 });
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: listProviders });
  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });

  useEffect(() => () => {
    if (highlightTimeoutRef.current) window.clearTimeout(highlightTimeoutRef.current);
  }, []);

  function handleTimelineSelect(index: number) {
    if (!scrollToDetailMessage(index)) return;
    setHighlightedIndex(index);
    if (highlightTimeoutRef.current) window.clearTimeout(highlightTimeoutRef.current);
    highlightTimeoutRef.current = window.setTimeout(() => setHighlightedIndex(null), 1200);
  }

  if (session.isLoading) return <PageSkeleton />;
  if (session.error) return <PageError title="Session failed to load" message={session.error.message} />;
  if (!session.data) return <PageEmpty title="Session not found" description="Return to the session list and choose another session." />;

  const { view, returned_event_count, has_more_events } = session.data;
  const localState = view.local_state ?? { archived: false, hidden: false, pinned: false, tags: [], preferred_targets: [], compressed_archive_refs: [] };
  const events = view.events ?? [];
  const artifacts = view.artifacts ?? [];
  const tags = localState.tags ?? [];
  const preferredTargets = localState.preferred_targets ?? [];
  const archives = (view.compressed_archive_refs ?? []).length || (localState.compressed_archive_refs ?? []).length;
  const actionTarget = { providerId: view.provider_id, sessionId: view.session_id, title: detailTitle(view), workspace: view.workspace_dir };
  const title = detailTitle(view);

  return (
    <>
      <ScrollArea className="h-full pr-3" data-session-detail-scroll>
        <div className="flex flex-col gap-4 pb-4">
          <DetailHeader
            data-session-header
            separated
            actionsPlacement="below"
            eyebrow={view.provider_name}
            title={formatDetailTitle(title)}
            meta={(
              <>
                <MetaField name="id" value={view.session_id} />
                <MetaField name="messages" value={String(view.message_count)} />
                <MetaField name="events" value={String(view.event_count)} />
                <MetaField name="loaded" value={String(returned_event_count)} />
                <MetaField name="artifacts" value={String(view.artifact_count)} />
                <MetaField name="archives" value={String(archives)} />
                <MetaField name="created" value={formatDateTime(view.created_at)} />
                <MetaField name="lastActive" value={formatDateTime(view.last_active_at)} />
                {view.workspace_dir ? <MetaField compact name="workspace" value={view.workspace_dir} /> : null}
                {view.source_path ? <MetaField compact name="sourcePath" value={view.source_path} /> : null}
                {view.resume_command ? <MetaField compact name="resumeCommand" value={view.resume_command} /> : null}
                {localState.notes ? <MetaField name="notes" value={localState.notes} /> : null}
                {localState.hidden ? <span>hidden</span> : null}
                {localState.pinned ? <span><PinIcon className="inline size-3" /> pinned</span> : null}
                {localState.archived ? <span><ArchiveIcon className="inline size-3" /> archived</span> : null}
                {tags.length ? <MetaField name="tags" value={tags.join(",")} /> : null}
                {preferredTargets.length ? <MetaField name="preferredTargets" value={preferredTargets.join(",")} /> : null}
                {has_more_events ? <span>more events available</span> : null}
              </>
            )}
            actions={(
              <>
              <Button type="button" variant="outline" onClick={() => setArtifactsOpen(true)}>Artifacts</Button>
              <Button type="button" variant="outline" onClick={() => setCompressionOpen(true)}>Compression</Button>
              <Button type="button" variant="outline" onClick={() => setSyncOpen(true)}>Sync</Button>
              <Button type="button" variant="outline" onClick={() => setSwitchOpen(true)}>Switch</Button>
              <Button type="button" variant="outline" onClick={() => setExportOpen(true)}>Export</Button>
              <Button type="button" variant="outline" onClick={() => setRenameOpen(true)}>Rename</Button>
              <Button type="button" variant="destructive" onClick={() => setDeleteOpen(true)}>Remove</Button>
              </>
            )}
          />

          <div className="grid min-h-0 gap-4 lg:grid-cols-[1.25rem_minmax(0,1fr)]" data-detail-layout>
            <DetailTimeline events={events} onScrollToMessage={handleTimelineSelect} />
            <div className="grid min-h-0 gap-2" data-session-message-list>
              {events.length === 0 ? (
                <PageEmpty title="No events" description="This session has no canonical events to render." />
              ) : (
                events.map((event, index) => (
                  <DetailEventItem
                    key={event.id}
                    event={event}
                    index={index}
                    highlighted={highlightedIndex === index}
                  />
                ))
              )}
            </div>
          </div>
        </div>
      </ScrollArea>
      <RenameSessionDialog
        open={renameOpen}
        target={actionTarget}
        onOpenChange={setRenameOpen}
      />
      <DeleteSessionDialog
        open={deleteOpen}
        target={actionTarget}
        onOpenChange={setDeleteOpen}
        returnHomeOnSuccess
      />
      <CompressSessionDialog
        open={compressionOpen}
        target={actionTarget}
        onOpenChange={setCompressionOpen}
      />
      <SwitchSessionDialog
        open={switchOpen}
        target={actionTarget}
        providers={providers.data ?? []}
        meta={meta.data}
        onOpenChange={setSwitchOpen}
      />
      <ExportSessionDialog
        open={exportOpen}
        target={actionTarget}
        meta={meta.data}
        onOpenChange={setExportOpen}
      />
      <CreateSyncDialog
        open={syncOpen}
        target={actionTarget}
        providers={providers.data ?? []}
        meta={meta.data}
        onOpenChange={setSyncOpen}
      />
      <SessionArtifactsDialog
        artifacts={artifacts}
        open={artifactsOpen}
        onOpenChange={setArtifactsOpen}
      />
    </>
  );
}
