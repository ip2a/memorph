import { useEffect, useMemo, useRef, useState } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ChevronDownIcon, CopyIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";
import { DetailHeader } from "@/components/shared/detail-header";
import { DetailTimeline, scrollToDetailMessage } from "@/components/shared/detail-timeline";
import { MetaLine } from "@/components/shared/meta-line";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { formatBytes, formatDateTime, formatDetailTitle, formatNumericDateTime } from "@/lib/format";
import type { SessionDetailView, SessionEvent, SessionEventOrder } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSession, useSessionActivity } from "@/features/sessions/queries";
import { SessionActivityChart } from "@/features/sessions/session-activity-chart";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { SessionEventBlocks } from "@/features/sessions/session-event-blocks";
import {
  eventBlockTagClass,
  eventKindTagClass,
  eventRoleTagClass,
  getBlockTags,
} from "@/features/sessions/session-block-utils";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/actions";
import { SessionDetailHeaderActions } from "@/features/sessions/session-detail-header-actions";
import { buildSessionEventQuery, sessionEventTotalPages, type SessionEventPageSize } from "@/features/sessions/session-detail-pagination";
import { readSessionDetailRouteState, writeSessionDetailRouteState } from "@/features/sessions/session-detail-route-state";
import { SessionDetailResultPagination } from "@/features/sessions/session-detail-result-pagination";
import { ApiError, getMeta, listProviders } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { useI18n } from "@/lib/i18n-context";

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

async function copyText(text: string, successMessage: string, errorMessage: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(successMessage);
  } catch {
    toast.error(errorMessage);
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
  const { t } = useI18n();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl" data-session-details-dialog>
        <DialogHeader>
          <DialogTitle>{t("sessionDetailsTitle")}</DialogTitle>
          <DialogDescription>{t("sessionDetailsDescription")}</DialogDescription>
        </DialogHeader>
        <ScrollArea className="max-h-[min(72vh,42rem)] pr-3">
          <div className="flex flex-col gap-5 text-sm">
            <section className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <strong>{t("sessionSnapshot")}</strong>
                {view.stale ? <Badge variant="destructive"><TriangleAlertIcon />{t("sessionStaleSource")}</Badge> : <Badge variant="secondary">{t("sessionFresh")}</Badge>}
              </div>
              <MetaLine columns="wide" label={t("sessionsMessages")} value={String(view.message_count)} />
              <MetaLine columns="wide" label={t("sessionEvents")} value={String(view.event_count)} />
              <MetaLine columns="wide" label={t("sessionTurns")} value={String(view.turns.length)} />
              <MetaLine
                columns="wide"
                label={t("sessionLoaded")}
                value={hasMoreEvents ? `${returnedEventCount} (${t("sessionMoreAvailable")})` : String(returnedEventCount)}
              />
              <MetaLine columns="wide" label={t("sessionArchives")} value={String(archives)} />
              <MetaLine columns="wide" label={t("sessionId")} value={<span className="break-all font-mono text-xs">{view.session_id}</span>} />
              <MetaLine columns="wide" label={t("sessionCreated")} value={formatDateTime(view.created_at)} />
              <MetaLine columns="wide" label={t("sessionLastActive")} value={formatDateTime(view.last_active_at)} />
              {view.source_path ? (
                <MetaLine
                  columns="wide"
                  label={t("sessionSourcePath")}
                  value={<PathText value={view.source_path} tone="default" wrap="all" className="text-sm" />}
                />
              ) : null}
              {localState.notes ? <MetaLine columns="wide" label={t("sessionNotes")} value={localState.notes} /> : null}
            </section>

            <section className="flex flex-col gap-3 border-t pt-4">
              <div className="flex flex-wrap items-center gap-2">
                <strong>{t("sessionProjectionQuality")}</strong>
                {view.projection_report ? (
                  <>
                    <Badge variant={qualityBadgeVariant(view.projection_report.status)}>
                      {readable(view.projection_report.status)}
                    </Badge>
                    <Badge variant={qualityBadgeVariant(view.projection_report.summary.mapping_overall)}>
                      {readable(view.projection_report.summary.mapping_overall)}
                    </Badge>
                  </>
                ) : <Badge variant="outline">{t("sessionNoReport")}</Badge>}
              </div>
              {view.projection_report ? (
                (() => {
                  const projectionItems = view.projection_report.items ?? [];
                  return (
                    <>
                      <div className="grid grid-cols-3 gap-2">
                        <StatItem label={t("sessionPreserved")} value={view.projection_report.summary.preserved_count} />
                        <StatItem label={t("sessionNormalized")} value={view.projection_report.summary.normalized_count} />
                        <StatItem label={t("sessionDropped")} value={view.projection_report.summary.dropped_count} />
                      </div>
                      <MetaLine columns="wide" label={t("sessionOperation")} value={readable(view.projection_report.operation_kind)} />
                      <MetaLine columns="wide" label={t("sessionVersion")} value={String(view.projection_report.projection_version)} />
                      <MetaLine columns="wide" label={t("sessionProjected")} value={formatDateTime(view.projection_report.created_at)} />
                      {projectionItems.length ? (
                        <div className="flex flex-col border-t">
                          {projectionItems.map((item) => (
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
                  );
                })()
              ) : null}
            </section>
            <section className="flex flex-col gap-3 border-t pt-4">
              <strong>{t("sessionTurns")}</strong>
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
              )) : <span className="text-muted-foreground">{t("sessionNoPersistedTurns")}</span>}
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
  const { t } = useI18n();
  const created = formatNumericDateTime(createdAt);
  const active = formatNumericDateTime(lastActiveAt);
  const timeLabel = lastActiveAt && active !== created ? `${created} · ${active}` : created;

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1" data-session-header-subtitle>
      <span className="shrink-0 font-mono text-xs tabular-nums text-muted-foreground">{timeLabel}</span>
      <code className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={sessionId}>
        {sessionId}
      </code>
      <Button type="button" variant="ghost" size="sm" className="h-6 shrink-0 px-2" onClick={() => copyText(sessionId, t("sessionCopied", { label: t("sessionId") }), t("sessionCopyFailed", { label: t("sessionId") }))}>
        <CopyIcon className="size-3.5" />
        {t("sessionCopy")}
      </Button>
    </div>
  );
}

function SessionDetailMeta({
  view,
  page,
  pageSize,
  matchedEventCount,
}: {
  view: SessionDetailView;
  page: number;
  pageSize: number;
  matchedEventCount?: number | null;
}) {
  const { t } = useI18n();
  const activity = useSessionActivity(view.provider_id, view.session_id);
  const searching = matchedEventCount != null;
  const totalEvents = searching ? matchedEventCount : view.event_count;
  const totalPages = sessionEventTotalPages(totalEvents, pageSize);
  const currentPage = Math.min(page, totalPages);
  const loadedFrom = totalEvents === 0 ? 0 : (currentPage - 1) * pageSize + 1;
  const loadedTo = Math.min(currentPage * pageSize, totalEvents);

  return (
    <div className="flex w-full flex-col gap-2.5" data-session-detail-meta>
      <div className="grid grid-cols-[minmax(0,0.34fr)_auto_minmax(0,1fr)] items-center gap-x-0 border-y py-2.5">
        <div className="flex min-w-0 flex-col justify-center gap-2.5 px-4 py-1 pr-3">
          <StatItem label={t("sessionProviderSource")} value={formatBytes(view.length_metrics.provider_source_bytes_measured)} title={t("sessionProviderSourceTooltip")} />
          <StatItem label={t("sessionModelVisible")} value={formatBytes(view.length_metrics.model_visible_bytes_measured)} title={t("sessionModelVisibleTooltip")} />
          <StatItem label={t("sessionEstimatedTokens")} value={view.length_metrics.estimated_tokens.toLocaleString()} title={t("sessionEstimatedTokensTooltip")} />
          <StatItem label={t("sessionMessagesEventsTurns")} value={`${view.length_metrics.message_count} / ${view.length_metrics.event_count} / ${view.length_metrics.turn_count}`} />
          <StatItem label={t("sessionCompressedArchives")} value={`${view.length_metrics.compressed_segment_count} / ${view.length_metrics.archive_count}`} />
          <StatItem
            label={searching ? t("sessionMatchesLoaded") : t("sessionLoaded")}
            value={totalEvents === 0 ? "0" : `${loadedFrom}–${loadedTo}${searching ? ` of ${matchedEventCount}` : ""}`}
            title={totalPages > 1 ? t("sessionPageOf", { page: currentPage, total: totalPages }) : undefined}
          />
        </div>

        <div className="mx-5 w-px self-stretch bg-border" aria-hidden />

        <SessionActivityChart
          className="min-h-[104px] min-w-0 overflow-visible pl-1"
          isLoading={activity.isLoading}
          timeline={activity.data}
        />
      </div>
    </div>
  );
}

function eventKindKey(event: SessionEvent) {
  return event.kind || "unknown";
}

function EventFoldDialog({
  open,
  onOpenChange,
  kinds,
  eventOrder,
  onEventOrderChange,
  onSetKindOpen,
  onSetAllOpen,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  kinds: Array<{ kind: string; count: number }>;
  eventOrder: SessionEventOrder;
  onEventOrderChange: (order: SessionEventOrder) => void;
  onSetKindOpen: (kind: string, open: boolean) => void;
  onSetAllOpen: (open: boolean) => void;
}) {
  const { t } = useI18n();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-session-event-fold-dialog>
        <DialogHeader>
          <DialogTitle>{t("sessionFilterEvents")}</DialogTitle>
          <DialogDescription>{t("sessionFilterDescription")}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <div className="text-sm font-medium">{t("sessionOrder")}</div>
            <div className="grid grid-cols-2 gap-2">
              <Button
                type="button"
                variant={eventOrder === "asc" ? "default" : "outline"}
                size="sm"
                onClick={() => onEventOrderChange("asc")}
              >
                {t("sessionOldestFirst")}
              </Button>
              <Button
                type="button"
                variant={eventOrder === "desc" ? "default" : "outline"}
                size="sm"
                onClick={() => onEventOrderChange("desc")}
              >
                {t("sessionNewestFirst")}
              </Button>
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
              <div className="text-sm font-medium">{t("sessionFoldByType")}</div>
              <div className="flex gap-2">
                <Button type="button" variant="outline" size="sm" onClick={() => onSetAllOpen(true)}>
                  {t("sessionExpandAll")}
                </Button>
                <Button type="button" variant="outline" size="sm" onClick={() => onSetAllOpen(false)}>
                  {t("sessionCollapseAll")}
                </Button>
              </div>
            </div>
            {kinds.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("sessionNoEventsOnPage")}</p>
            ) : (
              <div className="flex flex-col divide-y border-t">
                {kinds.map(({ kind, count }) => (
                  <div key={kind} className="flex items-center justify-between gap-3 py-3">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{readable(kind)}</div>
                      <div className="text-xs text-muted-foreground">{t("sessionEventCount", { count })}</div>
                    </div>
                    <div className="flex shrink-0 gap-2">
                      <Button type="button" variant="outline" size="sm" onClick={() => onSetKindOpen(kind, true)}>
                        {t("sessionExpand")}
                      </Button>
                      <Button type="button" variant="outline" size="sm" onClick={() => onSetKindOpen(kind, false)}>
                        {t("sessionCollapse")}
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function DetailEventItem({
  event,
  index,
  eventNumber,
  highlighted,
  open,
  onOpenChange,
}: {
  event: SessionEvent;
  index: number;
  eventNumber: number;
  highlighted?: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useI18n();
  const role = event.role ?? "unknown";
  const kind = event.kind ?? "unknown";
  const blockTags = getBlockTags(event.blocks, t);
  const blocks = event.blocks ?? [];

  return (
    <div className="min-w-0" data-message-index={index} data-session-event-row="single">
      <Collapsible open={open} onOpenChange={onOpenChange}>
        <article
          className={cn(
            "flex min-h-0 flex-col overflow-hidden rounded-xl border border-border bg-card",
            highlighted && "outline-2 outline-foreground/35 -outline-offset-2",
          )}
          data-event-number={eventNumber}
          data-role={role}
          data-kind={kind}
        >
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="flex w-full shrink-0 items-center justify-between gap-2 border-b px-2.5 py-2 text-left font-mono text-xs transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5 overflow-hidden">
                <span className="shrink-0 tabular-nums text-muted-foreground">#{eventNumber}</span>
                <Badge variant="outline" className={cn("uppercase", eventRoleTagClass(role))}>
                  {readable(role)}
                </Badge>
                <Badge variant="outline" className={eventKindTagClass(kind)}>
                  {readable(kind)}
                </Badge>
                {blockTags.map((tag) => (
                  <Badge key={`${tag.type}-${tag.label}`} variant="outline" className={eventBlockTagClass(tag.type)}>
                    {tag.label}
                  </Badge>
                ))}
                {event.tags?.map((tag) => (
                  <Badge key={tag} variant="outline" className="border-border bg-background font-normal text-muted-foreground">
                    {tag}
                  </Badge>
                ))}
                {event.metadata?.model ? (
                  <Badge variant="outline" className="border-border bg-background font-normal text-muted-foreground">
                    {event.metadata.model}
                  </Badge>
                ) : null}
                {event.metadata?.fidelity ? (
                  <Badge variant={qualityBadgeVariant(event.metadata.fidelity)}>{readable(event.metadata.fidelity)}</Badge>
                ) : null}
              </span>
              <span className="flex shrink-0 items-center gap-2">
                <span className="whitespace-nowrap text-muted-foreground">{formatDateTime(event.timestamp)}</span>
                <ChevronDownIcon
                  aria-hidden="true"
                  className={cn(
                    "size-3.5 text-muted-foreground transition-transform",
                    open && "rotate-180",
                  )}
                />
              </span>
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent className="overflow-hidden">
            <div className="p-3">
              {blocks.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("sessionNoDetails")}</p>
              ) : (
                <SessionEventBlocks blocks={blocks} eventId={event.id} />
              )}
            </div>
          </CollapsibleContent>
        </article>
      </Collapsible>
    </div>
  );
}

export function SessionDetailPage() {
  const { t } = useI18n();
  const [renameOpen, setRenameOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [compressionOpen, setCompressionOpen] = useState(false);
  const [switchOpen, setSwitchOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [syncOpen, setSyncOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [filterOpen, setFilterOpen] = useState(false);
  const [eventOpenById, setEventOpenById] = useState<Record<string, boolean>>({});
  const [eventSearchDraft, setEventSearchDraft] = useState("");
  const [searchSubmitPending, setSearchSubmitPending] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState<number | null>(null);
  const highlightTimeoutRef = useRef<number | null>(null);
  const { provider = "", sessionId = "" } = useParams();
  const [searchParams, setSearchParams] = useSearchParams();
  const route = useMemo(() => readSessionDetailRouteState(searchParams), [searchParams]);
  useEffect(() => {
    setEventSearchDraft(route.eventSearch);
  }, [route.eventSearch, provider, sessionId]);
  const sessionQuery = useMemo(
    () => buildSessionEventQuery(route.page, route.pageSize, route.eventSearch, route.eventOrder),
    [route.page, route.pageSize, route.eventSearch, route.eventOrder],
  );
  const session = useSession(provider, sessionId, sessionQuery);
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: listProviders });
  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });

  useEffect(() => () => {
    if (highlightTimeoutRef.current) window.clearTimeout(highlightTimeoutRef.current);
  }, []);

  useEffect(() => {
    if (!session.isFetching) setSearchSubmitPending(false);
  }, [session.isFetching]);

  useEffect(() => {
    setHighlightedIndex(null);
    setEventOpenById({});
  }, [provider, sessionId, route.page, route.pageSize, route.eventSearch, route.eventOrder]);

  useEffect(() => {
    if (!session.data) return;
    const paginationTotal = session.data.matched_event_count ?? session.data.view.event_count;
    const totalPages = sessionEventTotalPages(paginationTotal, route.pageSize);
    if (route.page <= totalPages) return;
    setSearchParams(writeSessionDetailRouteState(searchParams, { page: totalPages }), { replace: true });
  }, [route.page, route.pageSize, route.eventSearch, route.eventOrder, searchParams, session.data, setSearchParams]);

  const foldKinds = useMemo(() => {
    const counts = new Map<string, number>();
    for (const event of session.data?.view.events ?? []) {
      const kind = eventKindKey(event);
      counts.set(kind, (counts.get(kind) ?? 0) + 1);
    }
    return [...counts.entries()]
      .map(([kind, count]) => ({ kind, count }))
      .sort((a, b) => a.kind.localeCompare(b.kind));
  }, [session.data?.view.events]);

  function updateRoute(next: Partial<{ page: number; pageSize: SessionEventPageSize; eventSearch: string; eventOrder: SessionEventOrder }>) {
    setSearchParams(writeSessionDetailRouteState(searchParams, next));
  }

  function handleEventSearchSubmit() {
    setSearchSubmitPending(true);
    updateRoute({ eventSearch: eventSearchDraft.trim(), page: 1 });
    scrollSessionDetailToTop();
  }

  function handleTimelineSelect(index: number) {
    if (!scrollToDetailMessage(index)) return;
    setHighlightedIndex(index);
    if (highlightTimeoutRef.current) window.clearTimeout(highlightTimeoutRef.current);
    highlightTimeoutRef.current = window.setTimeout(() => setHighlightedIndex(null), 1200);
  }

  function scrollSessionDetailToTop() {
    const viewport = document.querySelector("[data-session-detail-scroll] [data-slot=scroll-area-viewport]");
    viewport?.scrollTo({ top: 0, behavior: "smooth" });
  }

  function changePage(page: number) {
    updateRoute({ page });
    scrollSessionDetailToTop();
  }

  function changePageSize(pageSize: SessionEventPageSize) {
    updateRoute({ page: 1, pageSize });
    scrollSessionDetailToTop();
  }

  function setOpenForKind(kind: string, open: boolean) {
    const list = session.data?.view.events ?? [];
    setEventOpenById((prev) => {
      const next = { ...prev };
      for (const event of list) {
        if (eventKindKey(event) === kind) next[event.id] = open;
      }
      return next;
    });
  }

  function setAllEventsOpen(open: boolean) {
    const list = session.data?.view.events ?? [];
    setEventOpenById((prev) => {
      const next = { ...prev };
      for (const event of list) next[event.id] = open;
      return next;
    });
  }

  if (session.isLoading && !session.data) return <PageSkeleton />;
  if (session.error) {
    const status = session.error instanceof ApiError ? session.error.status : 0;
    if (status === 410) return <PageError title={t("sessionSourceRemoved")} message={session.error.message} />;
    if (status === 501) return <PageError title={t("sessionDetailUnsupported")} message={session.error.message} />;
    if (status === 404) return <PageError title={t("sessionNotIndexed")} message={session.error.message} />;
    return <PageError title={t("sessionFailedToLoad")} message={session.error.message} />;
  }
  if (!session.data) return <PageEmpty title={t("sessionNotFound")} description={t("sessionNotFoundDescription")} />;

  const { view, returned_event_count, has_more_events, matched_event_count, returned_event_indices } = session.data;
  const localState = view.local_state ?? { archived: false, hidden: false, pinned: false, tags: [], preferred_targets: [], compressed_archive_refs: [] };
  const events = view.events ?? [];
  const eventOffset = session.data.events_offset ?? 0;
  const visibleEvents = events.map((event, index) => ({
    event,
    index,
    eventNumber: (returned_event_indices?.[index] ?? eventOffset + index) + 1,
  }));
  const paginationTotal = matched_event_count ?? view.event_count;
  const searching = Boolean(route.eventSearch.trim());
  const eventSearchPending = searchSubmitPending && session.isFetching;
  const archives = (view.compressed_archive_refs ?? []).length || (localState.compressed_archive_refs ?? []).length;
  const actionTarget = { providerId: view.provider_id, sessionId: view.session_id, title: detailTitle(view), workspace: view.workspace_dir };
  const title = detailTitle(view);

  return (
    <>
      <div className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden" data-session-detail-shell>
        <ScrollArea
          className="min-h-0 min-w-0 flex-1 overflow-hidden pr-3 [&_[data-slot=scroll-area-viewport]>div]:block [&_[data-slot=scroll-area-viewport]>div]:max-w-full [&_[data-slot=scroll-area-viewport]>div]:min-w-0"
          data-session-detail-scroll
        >
          <div className="flex min-w-0 max-w-full flex-col gap-4 pb-4">
            <DetailHeader
            data-session-header
            separated
            actionsPlacement="below"
            title={(
              <span className="inline-flex min-w-0 items-center gap-3">
                <ProviderLogo
                  providerId={view.provider_id}
                  size="sm"
                  alt={view.provider_name || view.provider_id}
                />
                <span className="min-w-0">{formatDetailTitle(title)}</span>
              </span>
            )}
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
                page={route.page}
                pageSize={route.pageSize}
                matchedEventCount={matched_event_count}
              />
            )}
            actions={(
              <SessionDetailHeaderActions
                eventSearchDraft={eventSearchDraft}
                onEventSearchDraftChange={setEventSearchDraft}
                onEventSearchSubmit={handleEventSearchSubmit}
                eventSearchPending={eventSearchPending}
                onOpenFilter={() => setFilterOpen(true)}
                onOpenDetails={() => setDetailsOpen(true)}
                onOpenCompression={() => setCompressionOpen(true)}
                onOpenSync={() => setSyncOpen(true)}
                onOpenSwitch={() => setSwitchOpen(true)}
                onOpenExport={() => setExportOpen(true)}
                onOpenRename={() => setRenameOpen(true)}
                onOpenDelete={() => setDeleteOpen(true)}
              />
            )}
            actionsProps={{ className: "w-full" }}
          />

          <div className="grid min-w-0 max-w-full grid-cols-[auto_minmax(0,1fr)] items-stretch gap-4" data-detail-layout>
            <DetailTimeline
              items={visibleEvents.map(({ event, index, eventNumber }) => ({ event, index, eventNumber }))}
              highlightedIndex={highlightedIndex}
              onScrollToMessage={(index) => handleTimelineSelect(index)}
            />
            <div
              className="grid min-h-0 min-w-0 gap-3"
              data-session-message-list
            >
              {!searching && view.event_count === 0 ? (
                <PageEmpty title={t("sessionNoEvents")} description={t("sessionNoEventsDescription")} />
              ) : searching && (matched_event_count ?? 0) === 0 ? (
                <PageEmpty
                  title={t("sessionNoMatchingEvents")}
                  description={t("sessionTryDifferentSearch")}
                />
              ) : (
                visibleEvents.map(({ event, index, eventNumber }) => (
                  <DetailEventItem
                    key={event.id}
                    event={event}
                    index={index}
                    eventNumber={eventNumber}
                    highlighted={highlightedIndex === index}
                    open={eventOpenById[event.id] ?? true}
                    onOpenChange={(open) => setEventOpenById((prev) => ({ ...prev, [event.id]: open }))}
                  />
                ))
              )}
            </div>
          </div>
          </div>
        </ScrollArea>
        {paginationTotal > 0 ? (
          <div
            className="shrink-0 border-t bg-background pr-3 pt-3 shadow-[0_-8px_16px_-12px_rgba(0,0,0,0.18)]"
            data-session-detail-pagination-bar
          >
            <SessionDetailResultPagination
              page={route.page}
              pageSize={route.pageSize}
              totalCount={paginationTotal}
              disabled={session.isFetching}
              onPageChange={changePage}
              onPageSizeChange={changePageSize}
            />
          </div>
        ) : null}
      </div>
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
      <SessionDetailsDialog
        open={detailsOpen}
        onOpenChange={setDetailsOpen}
        view={view}
        returnedEventCount={returned_event_count}
        hasMoreEvents={has_more_events}
        archives={archives}
        localState={localState}
      />
      <EventFoldDialog
        open={filterOpen}
        onOpenChange={setFilterOpen}
        kinds={foldKinds}
        eventOrder={route.eventOrder}
        onEventOrderChange={(eventOrder) => updateRoute({ eventOrder, page: 1 })}
        onSetKindOpen={setOpenForKind}
        onSetAllOpen={setAllEventsOpen}
      />
    </>
  );
}
