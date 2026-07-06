import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDownIcon } from "lucide-react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { updateSettings, updateWorkspaceProviders } from "@/lib/api";
import { formatBytes, formatDateTime, sessionTitle } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import { useUiStore } from "@/stores/ui-store";
import { useHomeData } from "@/features/home/queries";
import { homeProviderCandidates, resolveHomeProviders } from "@/features/home/model/providers";
import { randomAsciiBannerColor } from "@/features/home/ascii-banner";
import { HomeSessionListPanel, HomeSessionToolbar } from "@/features/home/home-session-controls";
import { HomeSessionGroupRail } from "@/features/home/home-session-group-rail";
import { ProviderActivitySparkline } from "@/features/home/provider-activity-sparkline";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import { targetFromSession } from "@/features/sessions/session-action-target";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/actions";
import type { HomeButtonSettingsPayload, SessionGroup, SessionHookFilter, SessionItem, SessionListSort, SettingsPayload, SyncGroup, UpdateSettingsPayload } from "@/lib/types";

type HomeButtons = Record<string, unknown> | undefined;

function homeButtonEnabled(homeButtons: HomeButtons, key: keyof HomeButtonSettingsPayload) {
  return homeButtons?.[key] !== false;
}

function clampSessionsPerProvider(value: number) {
  return Math.max(1, Math.min(200, Number(value || 6)));
}

function settingsPayloadFromMeta(settings: SettingsPayload, sessionsPerProvider: number): UpdateSettingsPayload {
  return {
    sessions_per_provider: clampSessionsPerProvider(sessionsPerProvider),
    language: settings.language ?? "auto",
    show_opencode_subagents: settings.show_opencode_subagents ?? false,
    sort_providers_by_session_count: settings.sort_providers_by_session_count ?? false,
    default_backup_dir: settings.default_backup_dir || "./backups",
    logging: {
      max_size_bytes: Number(settings.logging?.max_size_bytes ?? 5 * 1024 * 1024),
      retention_days: settings.logging?.retention_days == null ? null : Number(settings.logging.retention_days),
    },
    home_buttons: {
      view: settings.home_buttons?.view !== false,
      compress: settings.home_buttons?.compress !== false,
      switch: settings.home_buttons?.switch !== false,
      export: settings.home_buttons?.export !== false,
      sync: settings.home_buttons?.sync !== false,
      delete: settings.home_buttons?.delete !== false,
    },
    agent_order: settings.agent_order ?? [],
    primary_agents: settings.primary_agents ?? [],
  };
}

function matchesSearch(session: SessionItem, query: string) {
  if (!query) return true;
  const text = [session.session_id, session.title, session.native_title, session.display_title, session.project_dir]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return text.includes(query.toLowerCase());
}

const ASCII = `███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████  ██    ██  ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██  ██    ██  ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██  ██    ██  ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██`;

function totalSessions(groups: SessionGroup[]) {
  return groups.reduce((sum, group) => sum + group.sessions.length, 0);
}

function findSyncRef(syncGroups: SyncGroup[], providerId: string, sessionId: string) {
  return syncGroups.find((group) => group.holdings.some((holding) => holding.provider === providerId && holding.session_id === sessionId))?.id ?? null;
}

function HomeHero({ workspace }: { workspace: string | null | undefined }) {
  const collapsed = useUiStore((state) => state.homeHeroCollapsed);
  const setCollapsed = useUiStore((state) => state.setHomeHeroCollapsed);
  const setWorkspaceSwitchOpen = useUiStore((state) => state.setWorkspaceSwitchOpen);
  const [bannerColor, setBannerColor] = useState(randomAsciiBannerColor);
  const title = workspaceName(workspace, "memorph");
  const path = workspace || "-";

  function setHeroCollapsed(next: boolean) {
    setBannerColor(randomAsciiBannerColor());
    setCollapsed(next);
  }

  function openWorkspaceSwitch() {
    setWorkspaceSwitchOpen(true);
  }

  if (collapsed) {
    return (
      <section className="border-y py-2">
        <button
          type="button"
          className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-1 text-left hover:bg-muted"
          onClick={() => setHeroCollapsed(false)}
          aria-label="Expand workspace banner"
        >
          <span className="grid min-w-0 grid-cols-[auto_minmax(80px,auto)_minmax(120px,1fr)] items-baseline gap-2">
            <span className="font-mono text-xs uppercase text-muted-foreground">Workspace</span>
            <strong className="truncate">{title}</strong>
            <PathText value={path} fallback="-" wrap="truncate" className="min-w-0" />
          </span>
          <ChevronDownIcon aria-hidden="true" className="shrink-0" />
        </button>
      </section>
    );
  }

  return (
    <section className="grid grid-cols-1 items-start gap-3 overflow-hidden border-y py-3 md:grid-cols-[minmax(0,7fr)_minmax(220px,3fr)]">
      <button
        type="button"
        className="group grid min-w-0 place-items-center overflow-hidden rounded-md text-left hover:bg-muted/30"
        onClick={() => setHeroCollapsed(true)}
        title="Collapse"
        style={{ ["--ascii-banner-color" as string]: bannerColor }}
      >
        <pre
          className="m-0 overflow-hidden whitespace-pre font-mono text-[clamp(11px,1.18vw,16px)] font-black leading-none text-[var(--ascii-banner-color)] transition-opacity duration-150 group-hover:opacity-90"
          style={{ textShadow: "0 0 14px color-mix(in srgb, var(--ascii-banner-color) 28%, transparent)" }}
        >
          {ASCII}
        </pre>
      </button>
      <div className="flex min-w-0 flex-col items-start justify-start gap-2 self-stretch border-l pl-4">
        <h1 className="m-0 w-full truncate text-left text-2xl font-semibold leading-none">{title}</h1>
        <button
          type="button"
          className="w-full min-w-0 text-left hover:text-foreground hover:underline"
          onClick={openWorkspaceSwitch}
        >
          <PathText value={path} fallback="-" wrap="all" className="block w-full min-w-0 text-left" />
        </button>
      </div>
    </section>
  );
}

function SessionRow({
  session,
  syncGroups,
  homeButtons,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  session: SessionItem;
  syncGroups: SyncGroup[];
  homeButtons?: HomeButtons;
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  const syncRef = findSyncRef(syncGroups, session.provider_id, session.session_id);
  const detailHref = `/sessions/${encodeURIComponent(session.provider_id)}/${encodeURIComponent(session.session_id)}`;
  const showSync = homeButtonEnabled(homeButtons, "sync");

  return (
    <article className="grid min-h-14 border-b py-2.5 hover:bg-muted/60">
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
            <Link to={detailHref} className="min-w-0 truncate font-semibold hover:underline">
              {sessionTitle(session)}
            </Link>
            {syncRef ? (
              <Badge asChild className="shrink-0">
                <Link to={`/sync/${syncRef}`}>Active Sync</Link>
              </Badge>
            ) : null}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
            <Badge variant="outline" className="max-w-full font-mono">
              <span className="truncate">{session.session_id}</span>
            </Badge>
            <span className="shrink-0">{formatDateTime(session.last_active_at)}</span>
            <span className="shrink-0">{session.message_count ?? "-"} messages</span>
            <span className="shrink-0">{formatBytes(session.size_bytes)}</span>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          {homeButtonEnabled(homeButtons, "view") ? (
            <Button asChild variant="outline" size="sm">
              <Link to={detailHref}>View</Link>
            </Button>
          ) : null}
          {homeButtonEnabled(homeButtons, "compress") ? <Button type="button" variant="outline" size="sm" onClick={() => onCompress(session)}>Compression</Button> : null}
          {homeButtonEnabled(homeButtons, "switch") ? <Button type="button" variant="outline" size="sm" onClick={() => onSwitch(session)}>Switch</Button> : null}
          {homeButtonEnabled(homeButtons, "export") ? <Button type="button" variant="outline" size="sm" onClick={() => onExport(session)}>Export</Button> : null}
          {showSync && syncRef ? (
            <Button asChild variant="outline" size="sm">
              <Link to={`/sync/${syncRef}`}>Open Sync</Link>
            </Button>
          ) : showSync ? (
            <Button type="button" variant="outline" size="sm" onClick={() => onSync(session)}>Sync</Button>
          ) : null}
          <Button type="button" variant="outline" size="sm" onClick={() => onRename(session)}>Rename</Button>
          {homeButtonEnabled(homeButtons, "delete") ? (
            <Button type="button" variant="destructive" size="sm" onClick={() => onDelete(session)}>Remove</Button>
          ) : null}
        </div>
      </div>
    </article>
  );
}

function SessionGroups({
  groups,
  syncGroups,
  homeButtons,
  workspace,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  groups: SessionGroup[];
  syncGroups: SyncGroup[];
  homeButtons?: HomeButtons;
  workspace?: string | null;
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
    <div className="grid divide-y">
      {groups.map((group) => (
        <details key={group.provider_id} open data-home-session-group={group.provider_id}>
          <summary className="flex min-h-11 cursor-pointer items-center gap-3 px-1 font-mono font-bold">
            <span className="min-w-0 flex-1 truncate">{group.provider_name || group.provider_id}</span>
            <ProviderActivitySparkline providerId={group.provider_id} workspace={workspace} />
          </summary>
          <div className="grid border-t">
            {group.sessions.map((session) => (
              <SessionRow
                key={`${group.provider_id}:${session.session_id}`}
                session={session}
                syncGroups={syncGroups}
                homeButtons={homeButtons}
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
  const queryClient = useQueryClient();
  const [renameTarget, setRenameTarget] = useState<SessionItem | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<SessionItem | null>(null);
  const [compressionTarget, setCompressionTarget] = useState<SessionItem | null>(null);
  const [switchTarget, setSwitchTarget] = useState<SessionItem | null>(null);
  const [exportTarget, setExportTarget] = useState<SessionItem | null>(null);
  const [syncTarget, setSyncTarget] = useState<SessionItem | null>(null);
  const [selectedProviders, setSelectedProviders] = useState<string[]>([]);
  const [providersReady, setProvidersReady] = useState(false);
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SessionListSort>("recent");
  const [hookFilter, setHookFilter] = useState<SessionHookFilter>("all");
  const [sessionsPerProvider, setSessionsPerProvider] = useState(6);
  const selectedWorkspaceOverride = useUiStore((state) => state.selectedWorkspace);
  const { meta, providers, catalog, workspaceProviders, sessions, syncGroups } = useHomeData(
    selectedWorkspaceOverride || undefined,
    selectedProviders,
    { sort, hookFilter, sessionLimit: sessionsPerProvider },
  );
  const loading =
    meta.isLoading ||
    providers.isLoading ||
    catalog.isLoading ||
    workspaceProviders.isLoading ||
    syncGroups.isLoading;
  const shellError =
    meta.error || providers.error || catalog.error || workspaceProviders.error || syncGroups.error;

  const selectedWorkspace = selectedWorkspaceOverride || meta.data?.selected_workspace || null;
  const providerCandidates = useMemo(
    () => homeProviderCandidates(catalog.data?.providers ?? []),
    [catalog.data?.providers],
  );
  const defaultSessionsPerProvider = clampSessionsPerProvider(meta.data?.settings.sessions_per_provider ?? 12);

  useEffect(() => {
    if (meta.data?.settings.sessions_per_provider === undefined) return;
    // Sync external settings into local UI state. This effect only runs when the
    // backing setting changes, so it does not cause a render loop.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSessionsPerProvider(clampSessionsPerProvider(meta.data.settings.sessions_per_provider));
  }, [meta.data?.settings.sessions_per_provider]);

  useEffect(() => {
    // Workspace switch resets provider selection state. The flag is intentionally
    // reset before data refetches to avoid stale selections from showing.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setProvidersReady(false);
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSelectedProviders([]);
  }, [selectedWorkspace]);

  useEffect(() => {
    if (providersReady || catalog.isLoading || workspaceProviders.isLoading) return;
    // Derive the initial provider selection from workspace + catalog data once.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSelectedProviders(resolveHomeProviders(providerCandidates, workspaceProviders.data));
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setProvidersReady(true);
  }, [catalog.isLoading, providerCandidates, providersReady, workspaceProviders.data, workspaceProviders.isLoading]);

  const persistProviders = useMutation({
    mutationFn: (nextProviders: string[]) => {
      if (!selectedWorkspace) return Promise.resolve(nextProviders);
      return updateWorkspaceProviders(selectedWorkspace, nextProviders);
    },
    onSuccess: (saved) => {
      if (selectedWorkspace) {
        queryClient.setQueryData(queryKeys.workspaceProviders(selectedWorkspace), saved);
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot });
    },
  });

  const persistSessionsPerProvider = useMutation<SettingsPayload, Error, number>({
    mutationFn: (nextLimit: number) => {
      const settings = meta.data?.settings;
      if (!settings) return Promise.reject(new Error("Settings not loaded"));
      return updateSettings(settingsPayloadFromMeta(settings, nextLimit));
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.meta });
      void queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot });
    },
  });

  function applyFilters(next: {
    hookFilter: SessionHookFilter;
    selectedProviders: string[];
    sessionsPerProvider: number;
  }) {
    setHookFilter(next.hookFilter);
    const nextLimit = clampSessionsPerProvider(next.sessionsPerProvider);
    setSessionsPerProvider(nextLimit);
    const savedLimit = clampSessionsPerProvider(meta.data?.settings.sessions_per_provider ?? defaultSessionsPerProvider);
    if (meta.data?.settings && nextLimit !== savedLimit) {
      void persistSessionsPerProvider.mutate(nextLimit);
    }
    setSelectedProviders((current) => {
      if (
        current.length === next.selectedProviders.length &&
        current.every((id) => next.selectedProviders.includes(id))
      ) {
        return current;
      }
      void persistProviders.mutate(next.selectedProviders);
      return next.selectedProviders;
    });
  }

  const sessionGroups = useMemo(
    () =>
      (sessions.data ?? [])
        .map((group) => ({
          ...group,
          sessions: group.sessions.filter((session) => matchesSearch(session, search)),
        }))
        .filter((group) => group.sessions.length > 0),
    [search, sessions.data],
  );

  const listLoading = !providersReady || (Boolean(selectedProviders.length) && sessions.isLoading && !sessions.data);
  const listRefreshing = Boolean(selectedProviders.length) && sessions.isFetching && Boolean(sessions.data);
  const sessionCount = totalSessions(sessionGroups);
  const agentCount = sessionGroups.length;

  if (loading) return <PageSkeleton />;
  if (shellError) return <PageError title="Home data failed to load" message={shellError.message} />;

  const syncItems = syncGroups.data ?? [];
  const homeButtons = meta.data?.settings.home_buttons;

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3 overflow-hidden">
      <HomeHero workspace={selectedWorkspace} />

      <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] rounded-md border bg-background">
        <div className="flex items-center gap-3 border-b p-3">
          <strong className="shrink-0">Recent Sessions</strong>
          <HomeSessionToolbar
            className="min-w-0 flex-1"
            search={search}
            sort={sort}
            hookFilter={hookFilter}
            selectedProviders={selectedProviders}
            sessionsPerProvider={sessionsPerProvider}
            defaultSessionsPerProvider={defaultSessionsPerProvider}
            providerCandidates={providerCandidates}
            onSearchChange={setSearch}
            onSortChange={setSort}
            onFiltersApply={applyFilters}
          />
          <div className="flex shrink-0 items-center gap-2">
            <Button asChild variant="outline">
              <Link to="/manager">sessions={sessionCount}</Link>
            </Button>
            <Button asChild variant="outline">
              <Link to="/agents">agents={agentCount}</Link>
            </Button>
          </div>
        </div>
        <div className="grid min-h-0 grid-cols-[auto_minmax(0,1fr)] overflow-hidden">
          <HomeSessionGroupRail groups={sessionGroups} />
          <ScrollArea className="min-h-0" data-home-session-scroll>
            <HomeSessionListPanel
              loading={listLoading}
              refreshing={listRefreshing}
              errorMessage={sessions.error?.message}
            >
              <SessionGroups
                groups={sessionGroups}
                syncGroups={syncItems}
                homeButtons={homeButtons}
                workspace={selectedWorkspace}
                onRename={setRenameTarget}
                onDelete={setDeleteTarget}
                onCompress={setCompressionTarget}
                onSwitch={setSwitchTarget}
                onExport={setExportTarget}
                onSync={setSyncTarget}
              />
            </HomeSessionListPanel>
          </ScrollArea>
        </div>
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
