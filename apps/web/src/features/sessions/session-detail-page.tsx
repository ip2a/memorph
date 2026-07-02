import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeftIcon, ArchiveIcon, CopyIcon, PinIcon } from "lucide-react";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { compactPath, formatDateTime } from "@/lib/format";
import type { SessionDetailView, SessionEvent } from "@/lib/types";
import { useSession } from "@/features/sessions/queries";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { SessionBlock } from "@/features/sessions/session-block";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/session-actions";
import { getMeta, listProviders } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

function detailTitle(view: SessionDetailView) {
  return view.display_title || view.title || view.native_title || view.session_id;
}

function StatCard({ title, value, description }: { title: string; value: string | number; description: string }) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-semibold">{value}</div>
      </CardContent>
    </Card>
  );
}

function MetadataRow({ label, value }: { label: string; value: string | null | undefined }) {
  return (
    <div className="grid gap-1 md:grid-cols-[10rem_1fr]">
      <span className="text-muted-foreground">{label}</span>
      <span className="break-words font-medium">{value || "-"}</span>
    </div>
  );
}

function RoleBadge({ role }: { role: string }) {
  if (role === "assistant") return <Badge>Assistant</Badge>;
  if (role === "user") return <Badge variant="secondary">User</Badge>;
  if (role === "tool") return <Badge variant="outline">Tool</Badge>;
  return <Badge variant="outline">{role}</Badge>;
}

function FidelityBadge({ fidelity }: { fidelity: string }) {
  if (fidelity === "preserved") return <Badge variant="secondary">Preserved</Badge>;
  if (fidelity === "dropped" || fidelity === "unsupported") return <Badge variant="destructive">{fidelity}</Badge>;
  return <Badge variant="outline">{fidelity}</Badge>;
}

function UsageBadges({ event }: { event: SessionEvent }) {
  const usage = event.metadata?.usage;
  if (!usage) return null;

  return (
    <div className="flex flex-wrap gap-2">
      {usage.input_tokens === null || usage.input_tokens === undefined ? null : (
        <Badge variant="outline">Input {usage.input_tokens}</Badge>
      )}
      {usage.output_tokens === null || usage.output_tokens === undefined ? null : (
        <Badge variant="outline">Output {usage.output_tokens}</Badge>
      )}
      {usage.total_tokens === null || usage.total_tokens === undefined ? null : (
        <Badge variant="outline">Total {usage.total_tokens}</Badge>
      )}
    </div>
  );
}

function EventSection({ event, index }: { event: SessionEvent; index: number }) {
  const blocks = event.blocks ?? [];

  return (
    <section className="flex flex-col gap-4">
      {index > 0 ? <Separator /> : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <RoleBadge role={event.role ?? "unknown"} />
            <Badge variant="outline">{event.kind ?? "unknown"}</Badge>
            <FidelityBadge fidelity={event.metadata?.fidelity ?? "unknown"} />
          </div>
          <div className="break-words text-sm font-medium">{event.id}</div>
          <div className="text-muted-foreground">{formatDateTime(event.timestamp)}</div>
        </div>
        <UsageBadges event={event} />
      </div>
      {event.metadata?.model ? <Badge variant="outline">{event.metadata.model}</Badge> : null}
      <div className="flex flex-col gap-3">
        {blocks.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <CopyIcon />
              </EmptyMedia>
              <EmptyTitle>No blocks</EmptyTitle>
              <EmptyDescription>This event has metadata but no rendered content blocks.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          blocks.map((block, blockIndex) => <SessionBlock key={`${event.id}-${blockIndex}`} block={block} />)
        )}
      </div>
    </section>
  );
}

export function SessionDetailPage() {
  const [renameOpen, setRenameOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [compressionOpen, setCompressionOpen] = useState(false);
  const [switchOpen, setSwitchOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [syncOpen, setSyncOpen] = useState(false);
  const { provider = "", sessionId = "" } = useParams();
  const session = useSession(provider, sessionId, { event_limit: 80 });
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: listProviders });
  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });

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

  return (
    <>
      <ScrollArea className="h-full pr-3" data-session-detail-scroll>
        <div className="flex flex-col gap-6 pb-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-2">
              <div className="flex flex-wrap gap-2">
                <Badge variant="secondary">Session Detail</Badge>
                <Badge variant="outline">{view.provider_name}</Badge>
                {localState.pinned ? <Badge variant="secondary"><PinIcon />Pinned</Badge> : null}
                {localState.archived ? <Badge variant="outline"><ArchiveIcon />Archived</Badge> : null}
              </div>
              <h1 className="break-words text-3xl font-semibold">{detailTitle(view)}</h1>
              <p className="break-words text-muted-foreground">{view.session_id}</p>
            </div>
            <div className="flex flex-wrap justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCompressionOpen(true)}>Compression</Button>
              <Button type="button" variant="outline" onClick={() => setSyncOpen(true)}>Sync</Button>
              <Button type="button" variant="outline" onClick={() => setSwitchOpen(true)}>Switch</Button>
              <Button type="button" variant="outline" onClick={() => setExportOpen(true)}>Export</Button>
              <Button type="button" variant="outline" onClick={() => setRenameOpen(true)}>Rename</Button>
              <Button type="button" variant="destructive" onClick={() => setDeleteOpen(true)}>Remove</Button>
              <Button asChild variant="outline">
                <Link to="/sessions">
                  <ArrowLeftIcon data-icon="inline-start" />
                  Back
                </Link>
              </Button>
            </div>
          </div>

          <section className="grid gap-4 md:grid-cols-4">
            <StatCard title="Events" value={view.event_count} description={`${returned_event_count} loaded`} />
            <StatCard title="Messages" value={view.message_count} description="Canonical messages" />
            <StatCard title="Artifacts" value={view.artifact_count} description="Files and attachments" />
            <StatCard title="Archives" value={archives} description={has_more_events ? "More events available" : "Loaded page complete"} />
          </section>

          <Tabs defaultValue="events">
            <TabsList>
              <TabsTrigger value="events">Events</TabsTrigger>
              <TabsTrigger value="metadata">Metadata</TabsTrigger>
              <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
            </TabsList>

            <TabsContent value="events">
              <div className="flex flex-col gap-5">
                {events.length === 0 ? (
                  <PageEmpty title="No events" description="This session has no canonical events to render." />
                ) : (
                  events.map((event, index) => <EventSection key={event.id} event={event} index={index} />)
                )}
              </div>
            </TabsContent>

            <TabsContent value="metadata">
              <Card>
                <CardHeader>
                  <CardTitle>Session Metadata</CardTitle>
                  <CardDescription>Canonical identity, paths, and local state.</CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex flex-col gap-3">
                    <MetadataRow label="Provider" value={`${view.provider_name} (${view.provider_id})`} />
                    <MetadataRow label="Canonical ID" value={view.canonical_id} />
                    <MetadataRow label="Workspace" value={view.workspace_dir} />
                    <MetadataRow label="Source Path" value={view.source_path} />
                    <MetadataRow label="Resume Command" value={view.resume_command} />
                    <MetadataRow label="Created" value={formatDateTime(view.created_at)} />
                    <MetadataRow label="Last Active" value={formatDateTime(view.last_active_at)} />
                    <MetadataRow label="Notes" value={localState.notes} />
                    <Separator />
                    <div className="flex flex-wrap gap-2">
                      {localState.hidden ? <Badge variant="outline">Hidden</Badge> : null}
                      {tags.map((tag) => <Badge key={tag} variant="secondary">{tag}</Badge>)}
                      {preferredTargets.map((target) => <Badge key={target} variant="outline">{target}</Badge>)}
                    </div>
                  </div>
                </CardContent>
              </Card>
            </TabsContent>

            <TabsContent value="artifacts">
              <Card>
                <CardHeader>
                  <CardTitle>Artifacts</CardTitle>
                  <CardDescription>Files, images, patches, and attachments attached to the canonical session.</CardDescription>
                </CardHeader>
                <CardContent>
                  {artifacts.length === 0 ? (
                    <PageEmpty title="No artifacts" description="No session artifacts were found for this canonical session." />
                  ) : (
                    <ScrollArea className="max-h-[32rem]">
                      <Table>
                        <TableHeader>
                          <TableRow>
                            <TableHead>ID</TableHead>
                            <TableHead>Kind</TableHead>
                            <TableHead>Path</TableHead>
                            <TableHead>MIME</TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {artifacts.map((artifact) => (
                            <TableRow key={artifact.id}>
                              <TableCell className="font-medium">{artifact.id}</TableCell>
                              <TableCell><Badge variant="outline">{artifact.kind}</Badge></TableCell>
                              <TableCell>{compactPath(artifact.path)}</TableCell>
                              <TableCell>{artifact.mime_type ?? "-"}</TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    </ScrollArea>
                  )}
                </CardContent>
              </Card>
            </TabsContent>
          </Tabs>
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
    </>
  );
}
