import { useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArchiveIcon,
  CheckIcon,
  ChevronDownIcon,
  MoreHorizontalIcon,
  SearchIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { toast } from "sonner";
import {
  backupManagerItems,
  backupManagerWorkspace,
  cleanManagerItems,
  cleanManagerWorkspace,
} from "@/lib/api";
import { formatBytes, formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type {
  ManagerBackupResult,
  ManagerCleanResult,
  ManagerFilter,
  ManagerItem,
  ManagerWorkspaceItem,
  ProviderInfo,
} from "@/lib/types";
import {
  useManagerMeta,
  useManagerPreview,
  useManagerProviders,
  useManagerStats,
  useManagerWorkspaces,
} from "@/features/manager/queries";
import {
  readManagerRouteState,
  resolveManagerRequest,
} from "@/features/manager/manager-route-state";
import type {
  ManagerScope,
  ManagerSort,
  ManagerView,
} from "@/features/manager/manager-route-state";
import { useUiStore } from "@/stores/ui-store";

type ManagerActionTarget =
  | { kind: "delete-sessions"; items: ManagerItem[] }
  | { kind: "backup-sessions"; items: ManagerItem[] }
  | { kind: "delete-workspaces"; items: ManagerWorkspaceItem[] }
  | { kind: "backup-workspaces"; items: ManagerWorkspaceItem[] };

type ManagerActionReport = {
  title: string;
  summary: string;
  lines: string[];
};

function workspaceIdentity(item: ManagerWorkspaceItem) {
  return JSON.stringify([item.provider_id, item.workspace]);
}

function matchesSessionSearch(item: ManagerItem, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  return [
    item.title,
    item.session_id,
    item.provider_id,
    item.provider_name,
    item.project_dir,
    item.source_path,
  ].some((value) => value?.toLowerCase().includes(normalized));
}

function matchesWorkspaceSearch(item: ManagerWorkspaceItem, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  return [
    item.workspace,
    workspaceName(item.workspace),
    item.provider_id,
    item.provider_name,
  ].some((value) => value?.toLowerCase().includes(normalized));
}

function actionTitle(target: ManagerActionTarget | null) {
  switch (target?.kind) {
    case "delete-sessions":
      return target.items.length === 1 ? "Delete session" : "Delete sessions";
    case "backup-sessions":
      return target.items.length === 1 ? "Back up session" : "Back up sessions";
    case "delete-workspaces":
      return target.items.length === 1 ? "Delete workspace sessions" : "Delete workspace sessions";
    case "backup-workspaces":
      return target.items.length === 1 ? "Back up workspace" : "Back up workspaces";
    default:
      return "Manager action";
  }
}

function isBackupAction(target: ManagerActionTarget | null) {
  return target?.kind === "backup-sessions" || target?.kind === "backup-workspaces";
}

function isDeleteAction(target: ManagerActionTarget | null) {
  return target?.kind === "delete-sessions" || target?.kind === "delete-workspaces";
}

function actionStats(target: ManagerActionTarget | null) {
  if (!target) return { sessions: 0, workspaces: 0, bytes: 0 };
  if (target.kind === "delete-sessions" || target.kind === "backup-sessions") {
    return {
      sessions: target.items.length,
      workspaces: 0,
      bytes: target.items.reduce((sum, item) => sum + item.size_bytes, 0),
    };
  }
  return {
    sessions: target.items.reduce((sum, item) => sum + item.session_count, 0),
    workspaces: target.items.length,
    bytes: target.items.reduce((sum, item) => sum + item.total_size_bytes, 0),
  };
}

function cleanSummary(result: ManagerCleanResult) {
  return `${result.success} deleted, ${result.failed} failed, ${formatBytes(result.freed_bytes)} freed`;
}

function backupSummary(result: ManagerBackupResult) {
  return `${result.success} backed up, ${result.failed} failed`;
}

function ProviderFilter({
  providers,
  selected,
  onToggle,
  onSelectAll,
}: {
  providers: ProviderInfo[];
  selected: string[];
  onToggle: (providerId: string) => void;
  onSelectAll: () => void;
}) {
  const selectedNames = providers
    .filter((provider) => selected.includes(provider.id))
    .map((provider) => provider.name);
  const label =
    selectedNames.length === 0
      ? "All providers"
      : selectedNames.length === 1
        ? selectedNames[0]
        : `${selectedNames.length} providers`;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="outline"
          className="min-w-0 justify-between sm:min-w-44"
          data-manager-provider-filter
        >
          <span className="truncate">{label}</span>
          <ChevronDownIcon data-icon="inline-end" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        <DropdownMenuLabel>Provider</DropdownMenuLabel>
        <DropdownMenuCheckboxItem
          checked={selected.length === 0}
          onCheckedChange={onSelectAll}
        >
          All providers
        </DropdownMenuCheckboxItem>
        <DropdownMenuSeparator />
        {providers.map((provider) => (
          <DropdownMenuCheckboxItem
            key={provider.id}
            checked={selected.includes(provider.id)}
            onSelect={(event) => event.preventDefault()}
            onCheckedChange={() => onToggle(provider.id)}
          >
            {provider.name}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ScopeControl({
  scope,
  specifiedWorkspace,
  onChange,
}: {
  scope: ManagerScope;
  specifiedWorkspace: boolean;
  onChange: (scope: ManagerScope) => void;
}) {
  return (
    <div
      className="grid grid-cols-2 rounded-lg bg-muted p-1"
      data-manager-scope-control
      aria-label="Workspace scope"
    >
      <Button
        type="button"
        size="sm"
        variant={scope === "current" && !specifiedWorkspace ? "secondary" : "ghost"}
        aria-pressed={scope === "current" && !specifiedWorkspace}
        onClick={() => onChange("current")}
      >
        Current workspace
      </Button>
      <Button
        type="button"
        size="sm"
        variant={scope === "all" && !specifiedWorkspace ? "secondary" : "ghost"}
        aria-pressed={scope === "all" && !specifiedWorkspace}
        onClick={() => onChange("all")}
      >
        All workspaces
      </Button>
    </div>
  );
}

function ViewControl({
  view,
  onChange,
}: {
  view: ManagerView;
  onChange: (view: ManagerView) => void;
}) {
  return (
    <Tabs
      value={view}
      onValueChange={(value) => onChange(value as ManagerView)}
      data-manager-view-tabs
    >
      <TabsList className="grid w-full grid-cols-2 sm:w-72">
        <TabsTrigger value="sessions">Sessions</TabsTrigger>
        <TabsTrigger value="workspaces">Workspaces</TabsTrigger>
      </TabsList>
    </Tabs>
  );
}

function FilterToolbar({
  view,
  providers,
  selectedProviders,
  search,
  sort,
  visibleCount,
  hasActiveFilters,
  onToggleProvider,
  onSelectAllProviders,
  onSearchChange,
  onSortChange,
  onSelectVisible,
  onClearFilters,
}: {
  view: ManagerView;
  providers: ProviderInfo[];
  selectedProviders: string[];
  search: string;
  sort: ManagerSort;
  visibleCount: number;
  hasActiveFilters: boolean;
  onToggleProvider: (providerId: string) => void;
  onSelectAllProviders: () => void;
  onSearchChange: (value: string) => void;
  onSortChange: (sort: ManagerSort) => void;
  onSelectVisible: () => void;
  onClearFilters: () => void;
}) {
  return (
    <div
      className="grid gap-2 border-b pb-3 sm:grid-cols-[minmax(14rem,1fr)_auto_auto] xl:grid-cols-[minmax(18rem,1fr)_auto_auto_auto_auto]"
      data-manager-filter-toolbar
    >
      <div className="relative min-w-0 sm:col-span-3 xl:col-span-1">
        <SearchIcon
          className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Input
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={
            view === "sessions"
              ? "Search sessions, providers, or paths"
              : "Search workspaces, providers, or paths"
          }
          className="pl-8"
          data-manager-search
        />
      </div>

      <ProviderFilter
        providers={providers}
        selected={selectedProviders}
        onToggle={onToggleProvider}
        onSelectAll={onSelectAllProviders}
      />

      <Select value={sort} onValueChange={(value) => onSortChange(value as ManagerSort)}>
        <SelectTrigger className="min-w-36" data-manager-sort>
          <SelectValue placeholder="Sort" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="recent">Recent</SelectItem>
            <SelectItem value="size">Size</SelectItem>
            {view === "workspaces" ? (
              <SelectItem value="sessions">Session count</SelectItem>
            ) : null}
            <SelectItem value="title">{view === "sessions" ? "Title" : "Name"}</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>

      <Button
        type="button"
        variant="outline"
        disabled={visibleCount === 0}
        onClick={onSelectVisible}
        data-manager-select-visible
      >
        <CheckIcon data-icon="inline-start" />
        Select visible
      </Button>

      {hasActiveFilters ? (
        <Button
          type="button"
          variant="ghost"
          onClick={onClearFilters}
          data-manager-clear-filters
        >
          <XIcon data-icon="inline-start" />
          Clear filters
        </Button>
      ) : null}
    </div>
  );
}

function RowActions({
  href,
  label,
  onBackup,
  onDelete,
}: {
  href: string;
  label: string;
  onBackup: () => void;
  onDelete: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label={`More actions for ${label}`}
          data-manager-row-more
        >
          <MoreHorizontalIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem asChild>
          <Link to={href}>Open</Link>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onBackup}>
          <ArchiveIcon />
          Back up
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={onDelete}>
          <Trash2Icon />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SessionRows({
  items,
  selected,
  onToggle,
  onBackup,
  onDelete,
}: {
  items: ManagerItem[];
  selected: Set<string>;
  onToggle: (id: string) => void;
  onBackup: (item: ManagerItem) => void;
  onDelete: (item: ManagerItem) => void;
}) {
  if (!items.length) {
    return (
      <PageEmpty
        title="No sessions matched"
        description="Change the search or filters to see sessions in this scope."
      />
    );
  }

  return (
    <div className="flex flex-col gap-2" data-manager-session-list>
      {items.map((item) => {
        const label = item.title || item.session_id;
        const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
        const checked = selected.has(item.id);

        return (
          <article
            key={item.id}
            className="grid min-w-0 grid-cols-[2.5rem_minmax(0,1fr)_2.5rem] items-stretch rounded-lg border bg-card transition-colors hover:border-foreground/20 data-[selected=true]:bg-muted/40"
            data-manager-row
            data-selected={checked ? "true" : "false"}
          >
            <div className="grid place-items-center border-r">
              <Checkbox
                checked={checked}
                onCheckedChange={() => onToggle(item.id)}
                aria-label={`Select ${label}`}
              />
            </div>

            <Link
              to={href}
              className="flex min-w-0 flex-col gap-2 px-3 py-3 outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
              data-manager-row-link
            >
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <strong className="min-w-0 truncate text-sm font-medium">{label}</strong>
                <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
              </div>
              <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span>{formatBytes(item.size_bytes)}</span>
                <span>Updated {formatDateTime(item.last_active_at)}</span>
                <span className="truncate font-mono">{item.session_id}</span>
              </div>
              <PathText value={item.project_dir || item.source_path} wrap="all" />
            </Link>

            <div className="grid place-items-center border-l" data-manager-row-actions>
              <RowActions
                href={href}
                label={label}
                onBackup={() => onBackup(item)}
                onDelete={() => onDelete(item)}
              />
            </div>
          </article>
        );
      })}
    </div>
  );
}

function WorkspaceRows({
  items,
  selected,
  onToggle,
  onBackup,
  onDelete,
}: {
  items: ManagerWorkspaceItem[];
  selected: Set<string>;
  onToggle: (identity: string) => void;
  onBackup: (item: ManagerWorkspaceItem) => void;
  onDelete: (item: ManagerWorkspaceItem) => void;
}) {
  if (!items.length) {
    return (
      <PageEmpty
        title="No workspaces matched"
        description="Change the search or filters to see workspaces in this scope."
      />
    );
  }

  return (
    <div className="flex flex-col gap-2" data-manager-workspace-list>
      {items.map((item) => {
        const identity = workspaceIdentity(item);
        const label = workspaceName(item.workspace);
        const params = new URLSearchParams({
          view: "sessions",
          workspace: item.workspace,
          providers: item.provider_id,
        });
        const href = `/manager?${params.toString()}`;
        const checked = selected.has(identity);

        return (
          <article
            key={identity}
            className="grid min-w-0 grid-cols-[2.5rem_minmax(0,1fr)_2.5rem] items-stretch rounded-lg border bg-card transition-colors hover:border-foreground/20 data-[selected=true]:bg-muted/40"
            data-manager-row
            data-selected={checked ? "true" : "false"}
          >
            <div className="grid place-items-center border-r">
              <Checkbox
                checked={checked}
                onCheckedChange={() => onToggle(identity)}
                aria-label={`Select ${label}`}
              />
            </div>

            <Link
              to={href}
              className="flex min-w-0 flex-col gap-2 px-3 py-3 outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
              data-manager-row-link
            >
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <strong className="min-w-0 truncate text-sm font-medium">{label}</strong>
                <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
              </div>
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span>{item.session_count} sessions</span>
                <span>{formatBytes(item.total_size_bytes)}</span>
                <span>Updated {formatDateTime(item.last_active_at)}</span>
              </div>
              <span className="break-all font-mono text-xs text-muted-foreground">
                {item.workspace}
              </span>
            </Link>

            <div className="grid place-items-center border-l" data-manager-row-actions>
              <RowActions
                href={href}
                label={label}
                onBackup={() => onBackup(item)}
                onDelete={() => onDelete(item)}
              />
            </div>
          </article>
        );
      })}
    </div>
  );
}

function SelectionBar({
  count,
  visibleCount,
  bytes,
  onClear,
  onBackup,
  onDelete,
}: {
  count: number;
  visibleCount: number;
  bytes: number;
  onClear: () => void;
  onBackup: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/60 px-3 py-2"
      data-manager-selection-bar
    >
      <div className="mr-auto min-w-0">
        <strong className="text-sm">{count} selected</strong>
        <span className="ml-2 text-xs text-muted-foreground">
          {visibleCount} visible · {formatBytes(bytes)}
        </span>
      </div>
      <Button type="button" variant="ghost" onClick={onClear}>
        Clear
      </Button>
      <Button type="button" variant="outline" onClick={onBackup}>
        <ArchiveIcon data-icon="inline-start" />
        Back up
      </Button>
      <Button type="button" variant="destructive" onClick={onDelete}>
        <Trash2Icon data-icon="inline-start" />
        Delete
      </Button>
    </div>
  );
}

export function ManagerPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const route = useMemo(() => readManagerRouteState(searchParams), [searchParams]);
  const selectedWorkspaceOverride = useUiStore((state) => state.selectedWorkspace);
  const [selectedSessions, setSelectedSessions] = useState<Set<string>>(() => new Set());
  const [selectedWorkspaces, setSelectedWorkspaces] = useState<Set<string>>(() => new Set());
  const [actionTarget, setActionTarget] = useState<ManagerActionTarget | null>(null);
  const [actionReport, setActionReport] = useState<ManagerActionReport | null>(null);
  const queryClient = useQueryClient();
  const meta = useManagerMeta();
  const providers = useManagerProviders();
  const providerOptions = useMemo(
    () => (providers.data ?? []).filter((provider) => provider.scan),
    [providers.data],
  );
  const availableProviderIds = useMemo(
    () => new Set(providerOptions.map((provider) => provider.id)),
    [providerOptions],
  );
  const selectedProviders = useMemo(
    () =>
      providers.data
        ? route.providers.filter((providerId) => availableProviderIds.has(providerId))
        : route.providers,
    [availableProviderIds, providers.data, route.providers],
  );
  const currentWorkspace = selectedWorkspaceOverride || meta.data?.selected_workspace || null;
  const request = useMemo(
    () => resolveManagerRequest({ ...route, providers: selectedProviders }, currentWorkspace),
    [currentWorkspace, route, selectedProviders],
  );
  const filter: ManagerFilter = request.filter;
  const stats = useManagerStats(filter, { enabled: request.enabled });
  const sessions = useManagerPreview(filter, {
    enabled: request.enabled && route.view === "sessions",
  });
  const workspaces = useManagerWorkspaces(filter, {
    enabled: request.enabled && route.view === "workspaces",
  });

  useEffect(() => {
    if (!providers.data) return;
    const canonicalProviders = selectedProviders.join(",");
    if ((searchParams.get("providers") ?? "") === canonicalProviders) return;

    const next = new URLSearchParams(searchParams);
    if (canonicalProviders) next.set("providers", canonicalProviders);
    else next.delete("providers");
    setSearchParams(next, { replace: true });
  }, [providers.data, searchParams, selectedProviders, setSearchParams]);

  const managerAction = useMutation({
    mutationFn: async (target: ManagerActionTarget): Promise<ManagerActionReport> => {
      const outputDir = meta.data?.settings.default_backup_dir || "./backups";

      if (target.kind === "delete-sessions") {
        const result = await cleanManagerItems({ items: target.items });
        return {
          title: "Delete sessions",
          summary: cleanSummary(result),
          lines: result.errors || [],
        };
      }

      if (target.kind === "backup-sessions") {
        const result = await backupManagerItems({
          items: target.items,
          output_dir: outputDir,
        });
        return {
          title: "Back up sessions",
          summary: backupSummary(result),
          lines: [...(result.files || []), ...(result.errors || [])],
        };
      }

      if (target.kind === "delete-workspaces") {
        let success = 0;
        let failed = 0;
        let freed = 0;
        const lines: string[] = [];
        const errors: string[] = [];

        for (const item of target.items) {
          const result = await cleanManagerWorkspace({
            provider_id: item.provider_id,
            workspace: item.workspace,
          });
          success += result.success;
          failed += result.failed;
          freed += result.freed_bytes;
          lines.push(
            `${item.provider_id} / ${workspaceName(item.workspace)}: ${result.success} deleted, ${result.failed} failed`,
          );
          errors.push(...(result.errors || []));
        }

        return {
          title: "Delete workspace sessions",
          summary: cleanSummary({
            success,
            failed,
            freed_bytes: freed,
            errors,
          }),
          lines: [...lines, ...errors],
        };
      }

      let success = 0;
      let failed = 0;
      const lines: string[] = [];
      const files: string[] = [];
      const errors: string[] = [];

      for (const item of target.items) {
        const result = await backupManagerWorkspace({
          provider_id: item.provider_id,
          workspace: item.workspace,
          output_dir: outputDir,
        });
        success += result.success;
        failed += result.failed;
        lines.push(
          `${item.provider_id} / ${workspaceName(item.workspace)}: ${result.success} backed up, ${result.failed} failed`,
        );
        files.push(...(result.files || []));
        errors.push(...(result.errors || []));
      }

      return {
        title: "Back up workspaces",
        summary: backupSummary({ success, failed, files, errors }),
        lines: [...lines, ...files, ...errors],
      };
    },
    onSuccess: async (report) => {
      setActionTarget(null);
      setActionReport(report);
      setSelectedSessions(new Set());
      setSelectedWorkspaces(new Set());
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["manager"] }),
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
        queryClient.invalidateQueries({ queryKey: queryKeys.meta }),
      ]);
    },
    onError: (error) => {
      toast.error("Manager action failed", {
        description: error instanceof Error ? error.message : String(error),
      });
    },
  });

  if (providers.isLoading || meta.isLoading) return <PageSkeleton />;
  if (providers.error) {
    return (
      <PageError
        title="Manager providers failed to load"
        message={providers.error.message}
      />
    );
  }
  if (meta.error) {
    return (
      <PageError
        title="Manager workspace failed to load"
        message={meta.error.message}
      />
    );
  }
  if (stats.error) {
    return (
      <PageError title="Manager stats failed to load" message={stats.error.message} />
    );
  }
  if (sessions.error) {
    return (
      <PageError
        title="Manager sessions failed to load"
        message={sessions.error.message}
      />
    );
  }
  if (workspaces.error) {
    return (
      <PageError
        title="Manager workspaces failed to load"
        message={workspaces.error.message}
      />
    );
  }

  const sessionRows = sessions.data?.items ?? [];
  const workspaceRows = workspaces.data?.items ?? [];
  const visibleSessions = sessionRows.filter((item) =>
    matchesSessionSearch(item, route.search),
  );
  const visibleWorkspaces = workspaceRows.filter((item) =>
    matchesWorkspaceSearch(item, route.search),
  );
  const selectedSessionItems = sessionRows.filter((item) =>
    selectedSessions.has(item.id),
  );
  const selectedWorkspaceItems = workspaceRows.filter((item) =>
    selectedWorkspaces.has(workspaceIdentity(item)),
  );
  const selectedSessionBytes = selectedSessionItems.reduce(
    (sum, item) => sum + item.size_bytes,
    0,
  );
  const selectedWorkspaceBytes = selectedWorkspaceItems.reduce(
    (sum, item) => sum + item.total_size_bytes,
    0,
  );
  const visibleSelectedSessions = visibleSessions.filter((item) =>
    selectedSessions.has(item.id),
  ).length;
  const visibleSelectedWorkspaces = visibleWorkspaces.filter((item) =>
    selectedWorkspaces.has(workspaceIdentity(item)),
  ).length;
  const visibleRows = route.view === "sessions" ? visibleSessions : visibleWorkspaces;
  const totalCount =
    route.view === "sessions"
      ? sessions.data?.total_count
      : workspaces.data?.total_count;
  const totalSize =
    route.view === "sessions"
      ? sessions.data?.total_size_bytes
      : workspaces.data?.total_size_bytes;
  const selectedCount =
    route.view === "sessions"
      ? selectedSessionItems.length
      : selectedWorkspaceItems.length;
  const specifiedWorkspace = Boolean(route.workspace);
  const scopeTitle = specifiedWorkspace
    ? workspaceName(request.workspace || "")
    : route.scope === "all"
      ? "All workspaces"
      : "Current workspace";
  const scopeDescription = request.workspace
    ? request.workspace
    : route.scope === "all"
      ? "Sessions across every indexed workspace."
      : "No current workspace is selected.";
  const hasActiveFilters =
    selectedProviders.length > 0 || Boolean(route.search.trim()) || route.sort !== "recent";

  function replaceRoute(update: (next: URLSearchParams) => void, replace = false) {
    const next = new URLSearchParams(searchParams);
    update(next);
    setSearchParams(next, { replace });
  }

  function clearSelection() {
    setSelectedSessions(new Set());
    setSelectedWorkspaces(new Set());
  }

  function changeScope(scope: ManagerScope) {
    replaceRoute((next) => {
      next.delete("workspace");
      if (scope === "all") next.set("scope", "all");
      else next.delete("scope");
    });
    clearSelection();
  }

  function changeView(view: ManagerView) {
    replaceRoute((next) => {
      if (view === "workspaces") next.set("view", "workspaces");
      else next.delete("view");
      if (view === "sessions" && route.sort === "sessions") next.delete("sort");
    });
    clearSelection();
  }

  function changeSearch(value: string) {
    replaceRoute((next) => {
      if (value) next.set("q", value);
      else next.delete("q");
    }, true);
  }

  function changeSort(sort: ManagerSort) {
    replaceRoute((next) => {
      if (sort === "recent") next.delete("sort");
      else next.set("sort", sort);
    });
    clearSelection();
  }

  function setProviders(nextProviders: string[]) {
    replaceRoute((next) => {
      if (nextProviders.length) next.set("providers", nextProviders.join(","));
      else next.delete("providers");
    });
    clearSelection();
  }

  function toggleProvider(providerId: string) {
    setProviders(
      selectedProviders.includes(providerId)
        ? selectedProviders.filter((id) => id !== providerId)
        : [...selectedProviders, providerId],
    );
  }

  function clearFilters() {
    replaceRoute((next) => {
      next.delete("providers");
      next.delete("q");
      next.delete("sort");
    });
    clearSelection();
  }

  function toggleSession(id: string) {
    setSelectedSessions((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleWorkspace(identity: string) {
    setSelectedWorkspaces((current) => {
      const next = new Set(current);
      if (next.has(identity)) next.delete(identity);
      else next.add(identity);
      return next;
    });
  }

  function selectVisible() {
    if (route.view === "sessions") {
      setSelectedSessions(new Set(visibleSessions.map((item) => item.id)));
    } else {
      setSelectedWorkspaces(
        new Set(visibleWorkspaces.map((item) => workspaceIdentity(item))),
      );
    }
  }

  function openAction(target: ManagerActionTarget) {
    if (!target.items.length) {
      toast.error("No selection");
      return;
    }
    setActionTarget(target);
  }

  const pendingStats = actionStats(actionTarget);
  const backupDir = meta.data?.settings.default_backup_dir || "./backups";

  return (
    <>
      <div
        className="flex h-full min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-hidden"
        data-manager-page-layout
      >
        <section
          className="flex shrink-0 flex-col gap-3 rounded-lg border bg-card p-4"
          data-manager-page-context
        >
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-start">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <h1 className="text-lg font-semibold tracking-tight">Session manager</h1>
                <Badge variant={specifiedWorkspace ? "secondary" : "outline"}>
                  {specifiedWorkspace
                    ? "Specified workspace"
                    : route.scope === "all"
                      ? "All workspaces"
                      : "Current workspace"}
                </Badge>
              </div>
              <strong className="mt-2 block truncate text-sm">{scopeTitle}</strong>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {scopeDescription}
              </p>
            </div>
            <ScopeControl
              scope={route.scope}
              specifiedWorkspace={specifiedWorkspace}
              onChange={changeScope}
            />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3 border-t pt-3">
            <ViewControl view={route.view} onChange={changeView} />
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              {stats.isLoading ? (
                <Skeleton className="h-4 w-40" />
              ) : (
                <>
                  <span>{stats.data?.selected_agent_count ?? 0} providers</span>
                  <span>{totalCount ?? 0} total</span>
                  <span>{formatBytes(totalSize ?? 0)}</span>
                </>
              )}
            </div>
          </div>
        </section>

        <section
          className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-hidden rounded-lg border bg-card p-4"
          data-manager-result-panel
        >
          <FilterToolbar
            view={route.view}
            providers={providerOptions}
            selectedProviders={selectedProviders}
            search={route.search}
            sort={route.sort}
            visibleCount={visibleRows.length}
            hasActiveFilters={hasActiveFilters}
            onToggleProvider={toggleProvider}
            onSelectAllProviders={() => setProviders([])}
            onSearchChange={changeSearch}
            onSortChange={changeSort}
            onSelectVisible={selectVisible}
            onClearFilters={clearFilters}
          />

          {selectedCount > 0 ? (
            route.view === "sessions" ? (
              <SelectionBar
                count={selectedSessionItems.length}
                visibleCount={visibleSelectedSessions}
                bytes={selectedSessionBytes}
                onClear={() => setSelectedSessions(new Set())}
                onBackup={() =>
                  openAction({ kind: "backup-sessions", items: selectedSessionItems })
                }
                onDelete={() =>
                  openAction({ kind: "delete-sessions", items: selectedSessionItems })
                }
              />
            ) : (
              <SelectionBar
                count={selectedWorkspaceItems.length}
                visibleCount={visibleSelectedWorkspaces}
                bytes={selectedWorkspaceBytes}
                onClear={() => setSelectedWorkspaces(new Set())}
                onBackup={() =>
                  openAction({
                    kind: "backup-workspaces",
                    items: selectedWorkspaceItems,
                  })
                }
                onDelete={() =>
                  openAction({
                    kind: "delete-workspaces",
                    items: selectedWorkspaceItems,
                  })
                }
              />
            )
          ) : null}

          <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
            <span>
              {visibleRows.length} shown / {totalCount ?? 0} total
            </span>
            <span>
              Sorted by {route.sort === "sessions" ? "session count" : route.sort}
            </span>
          </div>

          {!request.enabled ? (
            <PageEmpty
              title="No current workspace"
              description="Choose a workspace from the app switcher or change the scope to All workspaces."
            />
          ) : route.view === "sessions" ? (
            <ScrollPane className="min-h-0 flex-1">
              {sessions.isLoading ? (
                <PageSkeleton />
              ) : (
                <SessionRows
                  items={visibleSessions}
                  selected={selectedSessions}
                  onToggle={toggleSession}
                  onBackup={(item) =>
                    openAction({ kind: "backup-sessions", items: [item] })
                  }
                  onDelete={(item) =>
                    openAction({ kind: "delete-sessions", items: [item] })
                  }
                />
              )}
            </ScrollPane>
          ) : (
            <ScrollPane className="min-h-0 flex-1">
              {workspaces.isLoading ? (
                <PageSkeleton />
              ) : (
                <WorkspaceRows
                  items={visibleWorkspaces}
                  selected={selectedWorkspaces}
                  onToggle={toggleWorkspace}
                  onBackup={(item) =>
                    openAction({ kind: "backup-workspaces", items: [item] })
                  }
                  onDelete={(item) =>
                    openAction({ kind: "delete-workspaces", items: [item] })
                  }
                />
              )}
            </ScrollPane>
          )}
        </section>
      </div>

      <Dialog
        open={Boolean(actionTarget)}
        onOpenChange={(open) => !open && setActionTarget(null)}
      >
        <DialogContent
          data-manager-action-dialog
          data-manager-delete-dialog={isDeleteAction(actionTarget) ? "true" : undefined}
          data-manager-backup-dialog={isBackupAction(actionTarget) ? "true" : undefined}
        >
          <DialogHeader>
            <DialogTitle>{actionTitle(actionTarget)}</DialogTitle>
            <DialogDescription>
              {isDeleteAction(actionTarget)
                ? "Confirm deletion of the selected sessions."
                : "Confirm backup of the selected sessions."}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2 rounded-md border p-3 font-mono text-xs">
            {pendingStats.workspaces > 0 ? (
              <span>Workspaces: {pendingStats.workspaces}</span>
            ) : null}
            <span>Sessions: {pendingStats.sessions}</span>
            <span>Estimated size: {formatBytes(pendingStats.bytes)}</span>
            {isBackupAction(actionTarget) ? <span>Backup dir: {backupDir}</span> : null}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setActionTarget(null)}
              disabled={managerAction.isPending}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant={isDeleteAction(actionTarget) ? "destructive" : "default"}
              onClick={() => actionTarget && managerAction.mutate(actionTarget)}
              disabled={!actionTarget || managerAction.isPending}
            >
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(actionReport)}
        onOpenChange={(open) => !open && setActionReport(null)}
      >
        <DialogContent data-manager-action-result>
          <DialogHeader>
            <DialogTitle>{actionReport?.title}</DialogTitle>
            <DialogDescription>{actionReport?.summary}</DialogDescription>
          </DialogHeader>
          {actionReport?.lines.length ? (
            <ScrollArea className="max-h-72 rounded-md border p-3">
              <pre className="whitespace-pre-wrap break-words font-mono text-xs">
                {actionReport.lines.join("\n")}
              </pre>
            </ScrollArea>
          ) : null}
          <DialogFooter>
            <Button type="button" onClick={() => setActionReport(null)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
