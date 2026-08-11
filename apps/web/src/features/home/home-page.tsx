import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDownIcon, RefreshCwIcon } from "lucide-react";
import { toast } from "sonner";
import { PageLoadError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import {
  TrailingMoreActionsDialog,
  type TrailingMoreAction,
} from "@/components/shared/trailing-more-actions-dialog";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { ensureReady, listSessions, updateSettings, updateWorkspaceProviders } from "@/lib/api";
import { formatBytes, formatDateTime, sessionTitle } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import { queryKeys } from "@/lib/query-keys";
import { useUiStore } from "@/stores/ui-store";
import { useHomeData } from "@/features/home/queries";
import { useReadiness } from "@/features/readiness/queries";
import { homeProviderCandidates, resolveHomeProviders } from "@/features/home/model/providers";
import { randomAsciiBannerColor } from "@/features/home/ascii-banner";
import { HomeSessionListPanel, HomeSessionToolbar } from "@/features/home/home-session-controls";
import { DEFAULT_SKILLS_CATALOG_PAGE_SIZE } from "@/features/skills/skills-catalog-page-size";
import { targetFromSession } from "@/features/sessions/session-action-target";
import { CreateSyncDialog, DeleteSessionDialog, ExportSessionDialog, RenameSessionDialog, SwitchSessionDialog } from "@/features/sessions/actions";
import { CompressSessionDialog } from "@/features/compression/compression-actions";
import type { HomeButtonSettingsPayload, HomeSessionLayout, SessionGroup, SessionItem, SessionListParams, SessionListSort, SettingsPayload, SyncGroup, UpdateSettingsPayload } from "@/lib/types";
import { cn } from "@/lib/utils";

type HomeButtons = Record<string, unknown> | undefined;

const DEFAULT_HOME_BUTTONS: Required<HomeButtonSettingsPayload> = {
  view: true,
  compress: false,
  switch: true,
  export: false,
  sync: false,
  rename: true,
  delete: true,
};

function homeButtonEnabled(homeButtons: HomeButtons, key: keyof HomeButtonSettingsPayload) {
  const configured = homeButtons?.[key];
  return typeof configured === "boolean" ? configured : DEFAULT_HOME_BUTTONS[key];
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
      compress: settings.home_buttons?.compress === true,
      switch: settings.home_buttons?.switch !== false,
      export: settings.home_buttons?.export === true,
      sync: settings.home_buttons?.sync === true,
      rename: settings.home_buttons?.rename !== false,
      delete: settings.home_buttons?.delete !== false,
    },
    home_session_layout: settings.home_session_layout ?? "tabs",
    skills_catalog_page_size: settings.skills_catalog_page_size ?? DEFAULT_SKILLS_CATALOG_PAGE_SIZE,
    agent_order: settings.agent_order ?? [],
    primary_agents: settings.primary_agents ?? [],
    server: {
      web_port: settings.server?.web_port ?? 3737,
      api_port: settings.server?.api_port ?? 3223,
    },
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

function findSyncRef(syncGroups: SyncGroup[], providerId: string, sessionId: string) {
  return syncGroups.find((group) => group.holdings.some((holding) => holding.provider === providerId && holding.session_id === sessionId))?.id ?? null;
}

function HomeHero({ workspace }: { workspace: string | null | undefined }) {
  const { t } = useI18n();
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
      <section className="-mt-3 border-b py-2">
        <button
          type="button"
          className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-1 text-left hover:bg-muted"
          onClick={() => setHeroCollapsed(false)}
          aria-label={t("expandWorkspaceBanner")}
        >
          <span className="grid min-w-0 grid-cols-[auto_minmax(80px,auto)_minmax(120px,1fr)] items-baseline gap-2">
            <span className="font-mono text-xs uppercase text-muted-foreground">{t("workspace")}</span>
            <strong className="truncate">{title}</strong>
            <PathText value={path} fallback="-" wrap="truncate" className="min-w-0" />
          </span>
          <ChevronDownIcon aria-hidden="true" className="shrink-0" />
        </button>
      </section>
    );
  }

  return (
    <section className="-mt-3 grid grid-cols-1 items-start gap-3 overflow-hidden border-b py-3 md:grid-cols-[minmax(0,7fr)_minmax(220px,3fr)]">
      <button
        type="button"
        className="group grid min-w-0 place-items-center overflow-hidden rounded-md text-left hover:bg-muted/30"
        onClick={() => setHeroCollapsed(true)}
        title={t("collapse")}
        style={{ ["--ascii-banner-color" as string]: bannerColor }}
      >
        <pre
          className="m-0 overflow-hidden whitespace-pre font-mono text-[clamp(11px,1.18vw,16px)] font-black leading-none text-[var(--ascii-banner-color)] transition-opacity duration-150 group-hover:opacity-90"
          style={{ textShadow: "0 0 14px color-mix(in srgb, var(--ascii-banner-color) 28%, transparent)" }}
        >
          {ASCII}
        </pre>
      </button>
      <div className="flex min-w-0 flex-col items-start justify-center gap-2 self-stretch border-l pl-4">
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

function renderSessionActionButton(action: TrailingMoreAction) {
  const variant = action.variant ?? "outline";

  if (action.href) {
    return (
      <Button key={action.id} asChild variant={variant}>
        <Link to={action.href}>{action.label}</Link>
      </Button>
    );
  }

  return (
    <Button key={action.id} type="button" variant={variant} onClick={action.onSelect}>
      {action.label}
    </Button>
  );
}

function SessionRowActions({
  session,
  detailHref,
  syncRef,
  homeButtons,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  session: SessionItem;
  detailHref: string;
  syncRef?: string;
  homeButtons?: HomeButtons;
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  const { t } = useI18n();
  const inlineEnabled = {
    view: homeButtonEnabled(homeButtons, "view"),
    compress: homeButtonEnabled(homeButtons, "compress"),
    switch: homeButtonEnabled(homeButtons, "switch"),
    export: homeButtonEnabled(homeButtons, "export"),
    sync: homeButtonEnabled(homeButtons, "sync"),
    rename: homeButtonEnabled(homeButtons, "rename"),
    remove: homeButtonEnabled(homeButtons, "delete"),
  };

  const actions: TrailingMoreAction[] = [
    { id: "view", label: t("view"), href: detailHref },
    {
      id: "compress",
      label: t("compression"),
      onSelect: () => onCompress(session),
    },
    {
      id: "switch",
      label: t("switch"),
      onSelect: () => onSwitch(session),
    },
    {
      id: "export",
      label: t("export"),
      onSelect: () => onExport(session),
    },
    syncRef
      ? { id: "sync", label: t("openSync"), href: `/sync?group=${encodeURIComponent(syncRef)}` }
      : {
          id: "sync",
          label: t("sync"),
          onSelect: () => onSync(session),
        },
    {
      id: "rename",
      label: t("rename"),
      onSelect: () => onRename(session),
    },
    {
      id: "remove",
      label: t("remove"),
      variant: "destructive",
      onSelect: () => onDelete(session),
    },
  ];
  const inlineActions = actions.filter((action) => inlineEnabled[action.id as keyof typeof inlineEnabled]);
  const leadingActions = inlineActions.slice(0, -1);
  const trailingAction = inlineActions.at(-1);

  const dialogTitle = t("moreActionsFor", { title: sessionTitle(session) });

  return (
    <div className="flex shrink-0 flex-wrap items-center justify-end gap-2" data-session-row-actions>
      {leadingActions.map(renderSessionActionButton)}
      <TrailingMoreActionsDialog
        trailingAction={trailingAction ? renderSessionActionButton(trailingAction) : null}
        moreLabel={dialogTitle}
        dialogTitle={dialogTitle}
        actions={actions}
      />
    </div>
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
  const { t } = useI18n();
  const syncRef = findSyncRef(syncGroups, session.provider_id, session.session_id);
  const detailHref = `/sessions/${encodeURIComponent(session.provider_id)}/${encodeURIComponent(session.session_id)}`;

  const copySessionId = async () => {
    try {
      await navigator.clipboard.writeText(session.session_id);
      toast.success(t("sessionCopied", { label: "ID" }));
    } catch {
      toast.error(t("sessionCopyFailed", { label: "ID" }));
    }
  };

  return (
    <article className="grid min-h-14 border-b py-2.5 hover:bg-muted/60">
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
        <div className="min-w-0 pl-2">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
            <Link to={detailHref} className="min-w-0 truncate font-semibold hover:underline">
              {sessionTitle(session)}
            </Link>
            {syncRef ? (
              <Badge asChild className="shrink-0">
                <Link to={`/sync?group=${encodeURIComponent(syncRef)}`}>{t("activeSync")}</Link>
              </Badge>
            ) : null}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
            <Badge
              variant="outline"
              className="max-w-full cursor-pointer font-mono"
              role="button"
              tabIndex={0}
              title={t("sessionCopy")}
              onClick={() => void copySessionId()}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  void copySessionId();
                }
              }}
            >
              <span className="truncate">{session.session_id}</span>
            </Badge>
            <span className="shrink-0">{formatDateTime(session.last_active_at)}</span>
            <span className="shrink-0">{session.message_count ?? "-"} {t("messages")}</span>
            <span className="shrink-0">{formatBytes(session.size_bytes)}</span>
          </div>
        </div>
        <SessionRowActions
          session={session}
          detailHref={detailHref}
          syncRef={syncRef ?? undefined}
          homeButtons={homeButtons}
          onRename={onRename}
          onDelete={onDelete}
          onCompress={onCompress}
          onSwitch={onSwitch}
          onExport={onExport}
          onSync={onSync}
        />
      </div>
    </article>
  );
}

function SessionGroupSessions({
  group,
  syncGroups,
  homeButtons,
  bordered = true,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  group: SessionGroup;
  syncGroups: SyncGroup[];
  homeButtons?: HomeButtons;
  bordered?: boolean;
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  return (
    <div className={cn("grid", bordered && "border-t")}>
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
  );
}

function SessionGroupsStack({
  groups,
  syncGroups,
  homeButtons,
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
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  return (
    <div className="grid divide-y">
      {groups.map((group) => (
        <details key={group.provider_id} open data-home-session-group={group.provider_id}>
          <summary className="flex min-h-11 cursor-pointer items-center gap-3 px-1 font-mono font-bold">
            <ProviderLogo providerId={group.provider_id} size="sm" alt={group.provider_name || group.provider_id} />
            <span className="min-w-0 flex-1 truncate">{group.provider_name || group.provider_id}</span>
          </summary>
          <SessionGroupSessions
            group={group}
            syncGroups={syncGroups}
            homeButtons={homeButtons}
            onRename={onRename}
            onDelete={onDelete}
            onCompress={onCompress}
            onSwitch={onSwitch}
            onExport={onExport}
            onSync={onSync}
          />
        </details>
      ))}
    </div>
  );
}

function SessionGroupsTabs({
  groups,
  syncGroups,
  homeButtons,
  isRefreshing,
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
  isRefreshing: boolean;
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  const { t } = useI18n();
  const [activeProviderId, setActiveProviderId] = useState(groups[0]?.provider_id ?? "");

  useEffect(() => {
    if (groups.some((group) => group.provider_id === activeProviderId)) return;
    setActiveProviderId(groups[0]?.provider_id ?? "");
  }, [activeProviderId, groups]);

  const activeGroup = groups.find((group) => group.provider_id === activeProviderId) ?? groups[0];

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-1 border-b" role="tablist" aria-label={t("homeSessionProviders")}>
        <div className="flex flex-wrap gap-1">
          {groups.map((group) => (
            <button
              key={group.provider_id}
              type="button"
              role="tab"
              aria-selected={activeProviderId === group.provider_id}
              data-home-session-group={group.provider_id}
              onClick={() => setActiveProviderId(group.provider_id)}
              className={cn(
                "flex items-center gap-2 border-b-2 px-3 py-2 text-sm font-medium transition-colors",
                activeProviderId === group.provider_id
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground",
              )}
            >
              <ProviderLogo providerId={group.provider_id} size="sm" alt={group.provider_name || group.provider_id} />
              <span className="font-mono">{group.provider_name || group.provider_id}</span>
            </button>
          ))}
        </div>
        {isRefreshing ? (
          <RefreshCwIcon className="mr-2 size-3.5 animate-spin text-muted-foreground" aria-label={t("scanning")} />
        ) : null}
      </div>
      {activeGroup ? (
        <SessionGroupSessions
          group={activeGroup}
          syncGroups={syncGroups}
          homeButtons={homeButtons}
          bordered={false}
          onRename={onRename}
          onDelete={onDelete}
          onCompress={onCompress}
          onSwitch={onSwitch}
          onExport={onExport}
          onSync={onSync}
        />
      ) : null}
    </div>
  );
}

function SessionGroups({
  layout,
  groups,
  syncGroups,
  homeButtons,
  isRefreshing,
  onRename,
  onDelete,
  onCompress,
  onSwitch,
  onExport,
  onSync,
}: {
  layout: HomeSessionLayout;
  groups: SessionGroup[];
  syncGroups: SyncGroup[];
  homeButtons?: HomeButtons;
  isRefreshing: boolean;
  onRename: (session: SessionItem) => void;
  onDelete: (session: SessionItem) => void;
  onCompress: (session: SessionItem) => void;
  onSwitch: (session: SessionItem) => void;
  onExport: (session: SessionItem) => void;
  onSync: (session: SessionItem) => void;
}) {
  const { t } = useI18n();
  if (!groups.length) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyTitle>{t("homeNoSessions")}</EmptyTitle>
          <EmptyDescription>{t("homeNoSessionsDescription")}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  const sharedProps = {
    groups,
    syncGroups,
    homeButtons,
    onRename,
    onDelete,
    onCompress,
    onSwitch,
    onExport,
    onSync,
  };

  if (layout === "tabs") {
    return <SessionGroupsTabs {...sharedProps} isRefreshing={isRefreshing} />;
  }

  return <SessionGroupsStack {...sharedProps} />;
}

export function HomePage() {
  const { t } = useI18n();
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
  const [sessionsPerProvider, setSessionsPerProvider] = useState(6);
  const selectedWorkspaceOverride = useUiStore((state) => state.selectedWorkspace);
  const readiness = useReadiness({ startOnMount: false });
  const { meta, providers, catalog, workspaceProviders, sessions, syncGroups, sessionParams } = useHomeData(
    selectedWorkspaceOverride || undefined,
    selectedProviders,
    { sort, sessionLimit: sessionsPerProvider },
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
    setSessionsPerProvider(clampSessionsPerProvider(meta.data.settings.sessions_per_provider));
  }, [meta.data?.settings.sessions_per_provider]);

  useEffect(() => {
    setProvidersReady(false);
    setSelectedProviders([]);
  }, [selectedWorkspace]);

  useEffect(() => {
    if (providersReady || catalog.isLoading || workspaceProviders.isLoading) return;
    setSelectedProviders(resolveHomeProviders(providerCandidates, workspaceProviders.data));
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
    selectedProviders: string[];
    sessionsPerProvider: number;
  }) {
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
      (sessions.data?.groups ?? [])
        .map((group) => ({
          ...group,
          sessions: group.sessions.filter((session) => matchesSearch(session, search)),
        }))
        .filter((group) => group.sessions.length > 0),
    [search, sessions.data],
  );
  const sessionScrollRef = useRef<HTMLDivElement | null>(null);
  const sessionScrollTopRef = useRef(0);

  useLayoutEffect(() => {
    const scrollPane = sessionScrollRef.current;
    if (scrollPane) scrollPane.scrollTop = sessionScrollTopRef.current;
  }, [sessionGroups]);

  const listLoading = !providersReady || (Boolean(selectedProviders.length) && sessions.isLoading && !sessions.data);
  const readinessRefreshing = readiness.isRunning || readiness.isReconciling;
  const listRefreshing = Boolean(selectedProviders.length) &&
    (readinessRefreshing || (sessions.isFetching && Boolean(sessions.data)));

  async function refreshSessions() {
    // Foundation repair stays as a lightweight guard; session discovery is now
    // handled by the workspace feed itself, which falls back to a full provider
    // scan when no lightweight scan is available.
    try {
      await ensureReady();
    } catch {
      // Continue so the feed can still report per-provider errors.
    }
    if (selectedWorkspace && selectedProviders.length > 0) {
      const refreshParams: SessionListParams = { ...sessionParams, refresh: true };
      await queryClient.fetchQuery({
        queryKey: queryKeys.sessionPage(refreshParams),
        queryFn: () => listSessions(refreshParams),
        staleTime: 0,
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.sessionPage(sessionParams) });
    }
    void meta.refetch();
    void syncGroups.refetch();
  }

  function retryHomeShell() {
    void meta.refetch();
    void providers.refetch();
    void catalog.refetch();
    void workspaceProviders.refetch();
    void syncGroups.refetch();
  }

  if (loading) return <PageSkeleton />;
  if (shellError) {
    return (
      <PageLoadError
        error={shellError}
        title={t("homeDataLoadFailed")}
        onRetry={retryHomeShell}
      />
    );
  }

  const syncItems = syncGroups.data ?? [];
  const homeButtons = meta.data?.settings.home_buttons;
  const homeSessionLayout = meta.data?.settings.home_session_layout ?? "tabs";

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3 overflow-hidden">
      <HomeHero workspace={selectedWorkspace} />

      <section className="flex h-full min-h-0 flex-col overflow-hidden rounded-md border bg-background">
        <div className="flex shrink-0 items-center gap-3 border-b p-3">
          <div className="flex shrink-0 items-center gap-2">
            <strong className="shrink-0">{t("recentSessions")}</strong>
          </div>
          <HomeSessionToolbar
            className="min-w-0 flex-1"
            search={search}
            sort={sort}
            selectedProviders={selectedProviders}
            sessionsPerProvider={sessionsPerProvider}
            defaultSessionsPerProvider={defaultSessionsPerProvider}
            providerCandidates={providerCandidates}
            onSearchChange={setSearch}
            onSortChange={setSort}
            onFiltersApply={applyFilters}
          />
          <Button
            type="button"
            variant="outline"
            disabled={!selectedProviders.length || sessions.isFetching || readinessRefreshing}
            onClick={refreshSessions}
          >
            <RefreshCwIcon
              className={listRefreshing ? "animate-spin" : undefined}
              data-icon="inline-start"
            />
            {t("refresh")}
          </Button>
        </div>
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <ScrollPane
            ref={sessionScrollRef}
            className="min-h-0 flex-1"
            innerClassName="min-h-full"
            data-home-session-scroll
            onScroll={(event) => {
              sessionScrollTopRef.current = event.currentTarget.scrollTop;
            }}
          >
            <HomeSessionListPanel
              loading={listLoading}
              errorMessage={sessions.error?.message}
            >
              <SessionGroups
                layout={homeSessionLayout}
                groups={sessionGroups}
                syncGroups={syncItems}
                homeButtons={homeButtons}
                isRefreshing={listRefreshing}
                onRename={setRenameTarget}
                onDelete={setDeleteTarget}
                onCompress={setCompressionTarget}
                onSwitch={setSwitchTarget}
                onExport={setExportTarget}
                onSync={setSyncTarget}
              />
            </HomeSessionListPanel>
          </ScrollPane>
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
