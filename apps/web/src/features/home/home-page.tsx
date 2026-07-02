import { useState } from "react";
import { Link } from "react-router-dom";
import { ChevronDownIcon, ChevronUpIcon, SearchIcon, SlidersHorizontalIcon } from "lucide-react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { compactPath, formatBytes, formatDateTime, sessionTitle } from "@/lib/format";
import { useUiStore } from "@/stores/ui-store";
import { useHomeData } from "@/features/home/queries";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { targetFromSession } from "@/features/sessions/session-action-target";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/session-actions";
import type { SessionGroup, SessionItem, SyncGroup } from "@/lib/types";

const ASCII = `███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████  ██    ██  ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██  ██    ██  ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██  ██    ██  ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██`;

function workspaceName(path: string | null | undefined) {
  if (!path) return "memorph";
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) || path;
}

function totalSessions(groups: SessionGroup[]) {
  return groups.reduce((sum, group) => sum + group.sessions.length, 0);
}

function findSyncRef(syncGroups: SyncGroup[], providerId: string, sessionId: string) {
  return syncGroups.find((group) => group.holdings.some((holding) => holding.provider === providerId && holding.session_id === sessionId))?.id ?? null;
}

function HomeHero({ workspace, groups }: { workspace: string | null | undefined; groups: SessionGroup[] }) {
  const collapsed = useUiStore((state) => state.homeHeroCollapsed);
  const setCollapsed = useUiStore((state) => state.setHomeHeroCollapsed);
  const sessionCount = totalSessions(groups);
  const title = workspaceName(workspace);
  const path = workspace || "-";

  if (collapsed) {
    return (
      <section className="border-y py-2">
        <button
          type="button"
          className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-1 text-left hover:bg-muted"
          onClick={() => setCollapsed(false)}
        >
          <span className="grid min-w-0 grid-cols-[auto_minmax(80px,auto)_minmax(120px,1fr)] items-baseline gap-2">
            <span className="font-mono text-xs uppercase text-muted-foreground">Workspace</span>
            <strong className="truncate">{title}</strong>
            <span className="truncate font-mono text-xs text-muted-foreground">{path}</span>
          </span>
          <span className="flex items-center gap-2 font-mono text-xs">
            <Link to="/manager" className="hover:underline" onClick={(event) => event.stopPropagation()}>
              sessions={sessionCount}
            </Link>
            <Link to="/agents" className="hover:underline" onClick={(event) => event.stopPropagation()}>
              agents={groups.length}
            </Link>
            <ChevronDownIcon aria-hidden="true" />
          </span>
        </button>
      </section>
    );
  }

  return (
    <section className="grid grid-cols-1 items-stretch gap-3 overflow-hidden border-y py-3 lg:grid-cols-[minmax(0,7fr)_minmax(220px,3fr)]">
      <button
        type="button"
        className="grid min-w-0 place-items-center overflow-hidden rounded-md text-left hover:bg-muted"
        onClick={() => setCollapsed(true)}
        title="Collapse"
      >
        <pre className="m-0 overflow-hidden whitespace-pre font-mono text-[clamp(11px,1.18vw,16px)] font-black leading-none text-primary">
          {ASCII}
        </pre>
      </button>
      <div className="grid min-w-0 content-center gap-2 border-l pl-4">
        <p className="m-0 font-mono text-xs uppercase text-muted-foreground">Workspace</p>
        <h1 className="m-0 truncate text-2xl font-semibold leading-none">{title}</h1>
        <Button type="button" variant="ghost" className="h-auto justify-start truncate px-0 py-0 font-mono text-xs text-muted-foreground">
          {path}
        </Button>
        <div className="flex flex-wrap gap-2 font-mono text-xs">
          <Badge asChild variant="outline">
            <Link to="/manager">sessions={sessionCount}</Link>
          </Badge>
          <Badge asChild variant="outline">
            <Link to="/agents">agents={groups.length}</Link>
          </Badge>
          <Badge variant="outline">shown={sessionCount}</Badge>
        </div>
        <Button type="button" variant="ghost" size="sm" className="w-fit" onClick={() => setCollapsed(true)}>
          <ChevronUpIcon data-icon="inline-start" />
          Collapse
        </Button>
      </div>
    </section>
  );
}

function ProviderPills({ groups }: { groups: SessionGroup[] }) {
  const visible = groups.slice(0, 6);
  const hidden = Math.max(0, groups.length - visible.length);

  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-start justify-center gap-2">
      {visible.map((group) => (
        <Badge key={group.provider_id} variant="outline" className="font-mono">
          {group.provider_name || group.provider_id}
        </Badge>
      ))}
      {hidden > 0 ? (
        <Button type="button" variant="outline" size="sm" className="rounded-full font-mono">
          More {hidden}
        </Button>
      ) : null}
    </div>
  );
}

function SessionRow({
  session,
  syncGroups,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  session: SessionItem;
  syncGroups: SyncGroup[];
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  const syncRef = findSyncRef(syncGroups, session.provider_id, session.session_id);
  const detailHref = `/sessions/${encodeURIComponent(session.provider_id)}/${encodeURIComponent(session.session_id)}`;

  return (
    <article className="grid min-h-14 border-b py-2.5 hover:bg-muted/60">
      <div className="grid min-w-0 grid-cols-1 items-start gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
        <div className="min-w-0">
          <div className="flex min-w-0 items-baseline gap-2">
            <Link to={detailHref} className="truncate font-semibold hover:underline">
              {sessionTitle(session)}
            </Link>
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
            <Badge variant="outline" className="max-w-full font-mono">
              <span className="truncate">{session.session_id}</span>
            </Badge>
            {syncRef ? (
              <Badge asChild>
                <Link to={`/sync/${syncRef}`}>Active Sync</Link>
              </Badge>
            ) : null}
            <span>{formatDateTime(session.last_active_at)}</span>
            <span>{session.message_count ?? "-"} messages</span>
            <span>{formatBytes(session.size_bytes)}</span>
            {session.project_dir ? <span className="truncate">{compactPath(session.project_dir)}</span> : null}
          </div>
        </div>
        <div className="flex flex-wrap justify-start gap-2 lg:justify-end">
          <Button asChild variant="outline" size="sm">
            <Link to={detailHref}>View</Link>
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => onCompress(session)}>Compression</Button>
          <Button type="button" variant="outline" size="sm" onClick={() => onSwitch(session)}>Switch</Button>
          <Button type="button" variant="outline" size="sm" onClick={() => onExport(session)}>Export</Button>
          {syncRef ? (
            <Button asChild variant="outline" size="sm">
              <Link to={`/sync/${syncRef}`}>Open Sync</Link>
            </Button>
          ) : (
            <Button type="button" variant="outline" size="sm" onClick={() => onSync(session)}>Sync</Button>
          )}
          <Button type="button" variant="outline" size="sm" onClick={() => onRename(session)}>Rename</Button>
          <Button type="button" variant="destructive" size="sm" onClick={() => onDelete(session)}>Remove</Button>
        </div>
      </div>
    </article>
  );
}

function SessionGroups({
  groups,
  syncGroups,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  groups: SessionGroup[];
  syncGroups: SyncGroup[];
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  if (!groups.length) {
    return (
      <Empty className="min-h-48 border">
        <EmptyHeader>
          <EmptyTitle>No sessions found</EmptyTitle>
          <EmptyDescription>Scan providers or switch workspace to show sessions here.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="grid">
      {groups.map((group) => (
        <details key={group.provider_id} className="border-t" open>
          <summary className="flex min-h-11 cursor-pointer items-center justify-between gap-3 font-mono font-bold">
            <span>{group.provider_name || group.provider_id}</span>
            <span>{group.sessions.length}/{group.sessions.length}</span>
          </summary>
          <div className="grid border-t">
            {group.sessions.map((session) => (
              <SessionRow
                key={`${group.provider_id}:${session.session_id}`}
                session={session}
                syncGroups={syncGroups}
                onRename={onRename}
                onDelete={onDelete}
                onCompress={onCompress}
                onSwitch={onSwitch}
                onExport={onExport}
                onSync={onSync}
              />
            ))}
          </div>
        </details>
      ))}
    </div>
  );
}

export function HomePage() {
  const [renameTarget, setRenameTarget] = useState<SessionItem | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<SessionItem | null>(null);
  const [compressionTarget, setCompressionTarget] = useState<SessionItem | null>(null);
  const [switchTarget, setSwitchTarget] = useState<SessionItem | null>(null);
  const [exportTarget, setExportTarget] = useState<SessionItem | null>(null);
  const [syncTarget, setSyncTarget] = useState<SessionItem | null>(null);
  const selectedWorkspaceOverride = useUiStore((state) => state.selectedWorkspace);
  const { meta, providers, sessions, syncGroups } = useHomeData(selectedWorkspaceOverride || undefined);
  const loading = meta.isLoading || providers.isLoading || sessions.isLoading || syncGroups.isLoading;
  const error = meta.error || providers.error || sessions.error || syncGroups.error;

  if (loading) return <PageSkeleton />;
  if (error) return <PageError title="Home data failed to load" message={error.message} />;

  const sessionGroups = sessions.data ?? [];
  const syncItems = syncGroups.data ?? [];
  const selectedWorkspace = selectedWorkspaceOverride || meta.data?.selected_workspace || null;

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3 overflow-hidden">
      <HomeHero workspace={selectedWorkspace} groups={sessionGroups} />

      <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] rounded-md border bg-background">
        <div className="flex items-center gap-3 border-b p-3">
          <div className="min-w-28">
            <strong>Recent Sessions</strong>
            <div className="font-mono text-xs uppercase text-muted-foreground">Filters</div>
          </div>
          <ProviderPills groups={sessionGroups} />
          <div className="relative min-w-52 flex-[0_1_300px]">
            <SearchIcon className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <Input className="pl-8" placeholder="Search sessions" />
          </div>
          <Button type="button" variant="outline" size="sm">Sort</Button>
          <Button type="button" variant="outline" size="sm">
            <SlidersHorizontalIcon data-icon="inline-start" />
            Filters
          </Button>
        </div>
        <ScrollArea className="min-h-0">
          <div className="px-3 pb-3">
            <SessionGroups
              groups={sessionGroups}
              syncGroups={syncItems}
              onRename={setRenameTarget}
              onDelete={setDeleteTarget}
              onCompress={setCompressionTarget}
              onSwitch={setSwitchTarget}
              onExport={setExportTarget}
              onSync={setSyncTarget}
            />
          </div>
        </ScrollArea>
      </section>
      <RenameSessionDialog
        open={Boolean(renameTarget)}
        target={renameTarget ? targetFromSession(renameTarget) : null}
        onOpenChange={(open) => {
          if (!open) setRenameTarget(null);
        }}
      />
      <DeleteSessionDialog
        open={Boolean(deleteTarget)}
        target={deleteTarget ? targetFromSession(deleteTarget) : null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      />
      <SwitchSessionDialog
        open={Boolean(switchTarget)}
        target={switchTarget ? targetFromSession(switchTarget) : null}
        providers={providers.data ?? []}
        meta={meta.data}
        onOpenChange={(open) => {
          if (!open) setSwitchTarget(null);
        }}
      />
      <CompressSessionDialog
        open={Boolean(compressionTarget)}
        target={compressionTarget ? targetFromSession(compressionTarget) : null}
        onOpenChange={(open) => {
          if (!open) setCompressionTarget(null);
        }}
      />
      <ExportSessionDialog
        open={Boolean(exportTarget)}
        target={exportTarget ? targetFromSession(exportTarget) : null}
        meta={meta.data}
        onOpenChange={(open) => {
          if (!open) setExportTarget(null);
        }}
      />
      <CreateSyncDialog
        open={Boolean(syncTarget)}
        target={syncTarget ? targetFromSession(syncTarget) : null}
        providers={providers.data ?? []}
        meta={meta.data}
        onOpenChange={(open) => {
          if (!open) setSyncTarget(null);
        }}
      />
    </div>
  );
}
