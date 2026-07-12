import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArchiveIcon, CopyIcon, InfoIcon, PinIcon, SearchIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";
import { DetailHeader } from "@/components/shared/detail-header";
import { DetailTimeline, scrollToDetailMessage } from "@/components/shared/detail-timeline";
import { MetaLine } from "@/components/shared/meta-line";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { formatDateTime, formatDetailTitle, formatNumericDateTime } from "@/lib/format";
import type { SessionArtifact, SessionDetailView, SessionEvent } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSession, useSessionActivity } from "@/features/sessions/queries";
import { SessionActivityChart } from "@/features/sessions/session-activity-chart";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { SessionBlock } from "@/features/sessions/session-block";
import { getBlockLabel } from "@/features/sessions/session-block-utils";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/actions";
import { getMeta, listProviders } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

function detailTitle(view: SessionDetailView) {
  return view.display_title || view.title || view.native_title || view.session_id;
}

function readable(value: string | null | undefined) {
  return value ? value.replaceAll("_", " ") : "-";
}

function qualityBadgeVariant(value: string | null | undefined): "secondary" | "outline" | "destructive" {
  if (value === "dropped" || value === "unsupported" || value === "failed" || value === "completed_with_loss") {
    return "destructive";
  }
  return value === "preserved" || value === "exact" || value === "completed" ? "secondary" : "outline";
}

async function copyText(text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`Copied ${label}`);
  } catch {
    toast.error(`Failed to copy ${label}`);
  }
}

function SessionDetailsDialog({
  open,
  onOpenChange,
  view,
  returnedEventCount,
  hasMoreEvents,
  archives,
  localState,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  view: SessionDetailView;
  returnedEventCount: number;
  hasMoreEvents: boolean;
  archives: number;
  localState: NonNullable<SessionDetailView["local_state"]>;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-session-details-dialog>
        <DialogHeader>
          <DialogTitle>Session details</DialogTitle>
          <DialogDescription>Snapshot state, projection quality, and persisted turn boundaries.</DialogDescription>
        </DialogHeader>
        <ScrollArea className="max-h-[min(72vh,42rem)] pr-3">
          <div className="flex flex-col gap-5 text-sm">
            <section className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <strong>Snapshot</strong>
                {view.stale ? <Badge variant="destructive"><TriangleAlertIcon />Stale source</Badge> : <Badge variant="secondary">Fresh</Badge>}
              </div>
              <MetaLine columns="wide" label="Messages" value={String(view.message_count)} />
              <MetaLine columns="wide" label="Events" value={String(view.event_count)} />
              <MetaLine columns="wide" label="Turns" value={String(view.turns.length)} />
              <MetaLine
                columns="wide"
                label="Loaded"
                value={hasMoreEvents ? `${returnedEventCount} (more available)` : String(returnedEventCount)}
              />
              <MetaLine columns="wide" label="Artifacts" value={String(view.artifact_count)} />
              <MetaLine columns="wide" label="Archives" value={String(archives)} />
              <MetaLine columns="wide" label="Session ID" value={<span className="break-all font-mono text-xs">{view.session_id}</span>} />
              <MetaLine columns="wide" label="Created" value={formatDateTime(view.created_at)} />
              <MetaLine columns="wide" label="Last active" value={formatDateTime(view.last_active_at)} />
              {view.source_path ? (
                <MetaLine
                  columns="wide"
                  label="Source path"
                  value={<PathText value={view.source_path} tone="default" wrap="all" className="text-sm" />}
                />
              ) : null}
              {localState.notes ? <MetaLine columns="wide" label="Notes" value={localState.notes} /> : null}
            </section>

            <section className="flex flex-col gap-3 border-t pt-4">
              <div className="flex flex-wrap items-center gap-2">
                <strong>Projection quality</strong>
                {view.projection_report ? (
                  <>
                    <Badge variant={qualityBadgeVariant(view.projection_report.status)}>
                      {readable(view.projection_report.status)}
                    </Badge>
                    <Badge variant={qualityBadgeVariant(view.projection_report.summary.mapping_overall)}>
                      {readable(view.projection_report.summary.mapping_overall)}
                    </Badge>
                  </>
                ) : <Badge variant="outline">No report</Badge>}
              </div>
              {view.projection_report ? (
                <>
                  <div className="grid grid-cols-3 gap-2">
                    <StatItem label="Preserved" value={view.projection_report.summary.preserved_count} />
                    <StatItem label="Normalized" value={view.projection_report.summary.normalized_count} />
                    <StatItem label="Dropped" value={view.projection_report.summary.dropped_count} />
                  </div>
                  <MetaLine columns="wide" label="Operation" value={readable(view.projection_report.operation_kind)} />
                  <MetaLine columns="wide" label="Version" value={String(view.projection_report.projection_version)} />
                  <MetaLine columns="wide" label="Projected" value={formatDateTime(view.projection_report.created_at)} />
                  {view.projection_report.items.length ? (
                    <div className="flex flex-col border-t">
                      {view.projection_report.items.map((item) => (
                        <div key={`${item.item_order}-${item.field_path ?? item.scope}`} className="grid gap-1 border-b py-2 sm:grid-cols-[auto_minmax(0,1fr)]">
                          <Badge variant={qualityBadgeVariant(item.fidelity)}>{readable(item.fidelity)}</Badge>
                          <div className="min-w-0">
                            <div className="break-all font-mono text-xs">{item.field_path || item.scope}</div>
                            {item.reason ? <div className="text-xs text-muted-foreground">{item.reason}</div> : null}
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : null}
                </>
              ) : null}
            </section>

            <section className="flex flex-col gap-3 border-t pt-4">
              <strong>Turns</strong>
              {view.turns.length ? view.turns.map((turn) => (
                <div key={turn.id} className="grid gap-2 border-b pb-3 sm:grid-cols-[auto_auto_minmax(0,1fr)] sm:items-center">
                  <span className="font-mono text-xs">#{turn.turn_order + 1}</span>
                  <div className="flex flex-wrap gap-1">
                    <Badge variant={qualityBadgeVariant(turn.confidence)}>{readable(turn.confidence)}</Badge>
                    <Badge variant={qualityBadgeVariant(turn.status)}>{readable(turn.status)}</Badge>
                  </div>
                  <div className="min-w-0 text-xs text-muted-foreground sm:text-right">
                    {formatDateTime(turn.started_at_ms)} to {formatDateTime(turn.ended_at_ms)}
                  </div>
                </div>
              )) : <span className="text-muted-foreground">No persisted turns.</span>}
            </section>
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  );
}

function StatItem({ label, value, title }: { label: string; value: number | string; title?: string }) {
  return (
    <div className="flex min-w-0 items-baseline justify-between gap-1 text-sm" title={title}>
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="font-semibold tabular-nums">{value}</span>
    </div>
  );
}

function SessionHeaderSubtitle({ sessionId, createdAt, lastActiveAt }: { sessionId: string; createdAt?: string | null; lastActiveAt?: string | null }) {
  const created = formatNumericDateTime(createdAt);
  const active = formatNumericDateTime(lastActiveAt);
  const timeLabel = lastActiveAt && active !== created ? `${created} · ${active}` : created;

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1" data-session-header-subtitle>
      <span className="shrink-0 font-mono text-xs tabular-nums text-muted-foreground">{timeLabel}</span>
      <code className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={sessionId}>
        {sessionId}
      </code>
      <Button type="button" variant="ghost" size="sm" className="h-6 shrink-0 px-2" onClick={() => copyText(sessionId, "session ID")}>
        <CopyIcon className="size-3.5" />
        Copy
      </Button>
    </div>
  );
}

function SessionDetailMeta({
  view,
  returnedEventCount,
  hasMoreEvents,
  localState,
}: {
  view: SessionDetailView;
  returnedEventCount: number;
  hasMoreEvents: boolean;
  localState: NonNullable<SessionDetailView["local_state"]>;
}) {
  const tags = localState.tags ?? [];
  const preferredTargets = localState.preferred_targets ?? [];
  const activity = useSessionActivity(view.provider_id, view.session_id);
  const confidenceCounts = view.turns.reduce<Record<string, number>>((counts, turn) => {
    counts[turn.confidence] = (counts[turn.confidence] ?? 0) + 1;
    return counts;
  }, {});

  return (
    <div className="flex w-full flex-col gap-2.5" data-session-detail-meta>
      <div className="grid gap-y-3 border-y py-2.5 lg:grid-cols-[minmax(0,0.34fr)_auto_minmax(0,1fr)] lg:items-center lg:gap-x-0">
        <div className="flex min-w-0 flex-col justify-center gap-2.5 px-4 py-1 lg:px-5 lg:pr-3">
          <StatItem label="Messages" value={view.message_count} />
          <StatItem label="Events" value={view.event_count} />
          <StatItem label="Loaded" value={returnedEventCount} title={hasMoreEvents ? "More events available beyond this page" : undefined} />
        </div>

        <div className="hidden w-px self-stretch bg-border lg:mx-5 lg:block" aria-hidden />

        <SessionActivityChart
          className="min-h-[104px] min-w-0 overflow-visible lg:pl-1"
          isLoading={activity.isLoading}
          timeline={activity.data}
        />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {view.resume_command ? (
          <Button type="button" variant="outline" size="sm" onClick={() => copyText(view.resume_command!, "resume command")}>
            <CopyIcon data-icon="inline-start" />
            Copy resume command
          </Button>
        ) : null}
        {view.stale ? <Badge variant="destructive"><TriangleAlertIcon className="size-3" />Stale source</Badge> : null}
        {view.projection_report ? (
          <Badge variant={qualityBadgeVariant(view.projection_report.summary.mapping_overall)}>
            Projection: {readable(view.projection_report.summary.mapping_overall)}
          </Badge>
        ) : null}
        {Object.entries(confidenceCounts).map(([confidence, count]) => (
          <Badge key={confidence} variant={qualityBadgeVariant(confidence)}>
            {count} {readable(confidence)} turns
          </Badge>
        ))}
        {localState.hidden ? <Badge variant="outline">Hidden</Badge> : null}
        {localState.pinned ? (
          <Badge variant="secondary">
            <PinIcon className="size-3" />
            Pinned
          </Badge>
        ) : null}
        {localState.archived ? (
          <Badge variant="secondary">
            <ArchiveIcon className="size-3" />
            Archived
          </Badge>
        ) : null}
        {tags.map((tag) => (
          <Badge key={tag} variant="outline">{tag}</Badge>
        ))}
        {preferredTargets.map((target) => (
          <Badge key={target} variant="outline">{target}</Badge>
        ))}
      </div>
    </div>
  );
}

function getBlockLabels(blocks: SessionEvent["blocks"]) {
  return (blocks ?? []).map(getBlockLabel).filter(Boolean);
}

function matchesEventSearch(event: SessionEvent, query: string) {
  if (!query.trim()) return true;
  const haystack = [
    event.id,
    event.role,
    event.kind,
    event.metadata?.model,
    JSON.stringify(event.blocks ?? []),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(query.trim().toLowerCase());
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
          {event.metadata?.fidelity ? (
            <Badge variant={qualityBadgeVariant(event.metadata.fidelity)}>{readable(event.metadata.fidelity)}</Badge>
          ) : null}
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
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [eventSearch, setEventSearch] = useState("");
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
  const visibleEvents = events
    .map((event, index) => ({ event, index }))
    .filter(({ event }) => matchesEventSearch(event, eventSearch));
  const artifacts = view.artifacts ?? [];
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
            title={formatDetailTitle(title)}
            description={(
              <SessionHeaderSubtitle
                sessionId={view.session_id}
                createdAt={view.created_at}
                lastActiveAt={view.last_active_at}
              />
            )}
            meta={(
              <SessionDetailMeta
                view={view}
                returnedEventCount={returned_event_count}
                hasMoreEvents={has_more_events}
                localState={localState}
              />
            )}
            actions={(
              <div className="flex w-full min-w-0 items-center gap-3">
                <div className="flex flex-wrap gap-2">
                  <Button type="button" variant="outline" onClick={() => setDetailsOpen(true)}>
                    <InfoIcon data-icon="inline-start" />
                    Details
                  </Button>
                  <Button type="button" variant="outline" onClick={() => setArtifactsOpen(true)}>Artifacts</Button>
                  <Button type="button" variant="outline" onClick={() => setCompressionOpen(true)}>Compression</Button>
                  <Button type="button" variant="outline" onClick={() => setSyncOpen(true)}>Sync</Button>
                  <Button type="button" variant="outline" onClick={() => setSwitchOpen(true)}>Switch</Button>
                  <Button type="button" variant="outline" onClick={() => setExportOpen(true)}>Export</Button>
                  <Button type="button" variant="outline" onClick={() => setRenameOpen(true)}>Rename</Button>
                  <Button type="button" variant="destructive" onClick={() => setDeleteOpen(true)}>Remove</Button>
                </div>
                <div className="relative min-w-[10rem] flex-1">
                  <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    className="h-8 pl-8"
                    value={eventSearch}
                    onChange={(event) => setEventSearch(event.target.value)}
                    placeholder="Search events"
                    data-session-event-search
                  />
                </div>
              </div>
            )}
            actionsProps={{ className: "w-full" }}
          />

          <div className="grid min-h-0 gap-4 lg:grid-cols-[1.25rem_minmax(0,1fr)]" data-detail-layout>
            <DetailTimeline events={visibleEvents.map(({ event }) => event)} onScrollToMessage={(index) => handleTimelineSelect(visibleEvents[index]?.index ?? index)} />
            <div className="grid min-h-0 gap-2" data-session-message-list>
              {events.length === 0 ? (
                <PageEmpty title="No events" description="This session has no canonical events to render." />
              ) : visibleEvents.length === 0 ? (
                <PageEmpty title="No matching events" description="Try a different search term." />
              ) : (
                visibleEvents.map(({ event, index }) => (
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
      <SessionDetailsDialog
        open={detailsOpen}
        onOpenChange={setDetailsOpen}
        view={view}
        returnedEventCount={returned_event_count}
        hasMoreEvents={has_more_events}
        archives={archives}
        localState={localState}
      />
    </>
  );
}
