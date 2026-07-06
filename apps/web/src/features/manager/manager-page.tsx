import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArrowRightIcon,
  CheckIcon,
  ChevronDownIcon,
  CopyIcon,
  FilterIcon,
  MoreHorizontalIcon,
  SearchIcon,
  Trash2Icon,
} from "lucide-react";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "sonner";
import {
  backupManagerItems,
  backupManagerWorkspace,
  cleanManagerItems,
  cleanManagerWorkspace,
} from "@/lib/api";
import { formatBytes, formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type { ManagerBackupResult, ManagerCleanResult, ManagerFilter, ManagerItem, ManagerWorkspaceItem, ProviderInfo } from "@/lib/types";
import {
  useManagerMeta,
  useManagerPreview,
  useManagerProviders,
  useManagerStats,
  useManagerWorkspaces,
} from "@/features/manager/queries";

type ManagerView = "sessions" | "workspaces";

type ManagerActionTarget =
  | { kind: "clean-sessions"; items: ManagerItem[] }
  | { kind: "backup-sessions"; items: ManagerItem[] }
  | { kind: "clean-workspaces"; items: ManagerWorkspaceItem[] }
  | { kind: "backup-workspaces"; items: ManagerWorkspaceItem[] };

type ManagerActionReport = {
  title: string;
  summary: string;
  lines: string[];
};

function sessionKey(item: ManagerItem) {
  return `${item.provider_id}:${item.session_id}`;
}

function workspaceKey(item: ManagerWorkspaceItem) {
  return `${item.provider_id}:${item.workspace}`;
}

function providerOptions(providers: ProviderInfo[] | undefined) {
  return (providers ?? []).filter((provider) => provider.scan);
}

function selectionSummary(
  total: number | undefined,
  visible: number,
  selected: number,
  bytes: number,
  searchActive: boolean,
) {
  const visibleLabel = searchActive ? `${visible} shown / ${total ?? 0} total` : `${total ?? 0} total`;
  return `${visibleLabel} / ${selected} selected / ${formatBytes(bytes)} selected`;
}

function matchesSessionSearch(item: ManagerItem, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  return [item.title, item.session_id, item.provider_id, item.provider_name, item.project_dir, item.source_path].some(
    (value) => value?.toLowerCase().includes(normalized),
  );
}

function matchesWorkspaceSearch(item: ManagerWorkspaceItem, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  return [item.workspace, workspaceName(item.workspace), item.provider_id, item.provider_name].some((value) =>
    value?.toLowerCase().includes(normalized),
  );
}

function actionTitle(target: ManagerActionTarget | null) {
  switch (target?.kind) {
    case "clean-sessions":
      return "Clean Selected";
    case "backup-sessions":
      return "Backup Selected";
    case "clean-workspaces":
      return "Clean Workspace Selected";
    case "backup-workspaces":
      return "Backup Workspace Selected";
    default:
      return "Manager Action";
  }
}

function isBackupAction(target: ManagerActionTarget | null) {
  return target?.kind === "backup-sessions" || target?.kind === "backup-workspaces";
}

function isCleanAction(target: ManagerActionTarget | null) {
  return target?.kind === "clean-sessions" || target?.kind === "clean-workspaces";
}

function actionStats(target: ManagerActionTarget | null) {
  if (!target) return { sessions: 0, workspaces: 0 };
  if (target.kind === "clean-sessions" || target.kind === "backup-sessions") {
    return { sessions: target.items.length, workspaces: 0 };
  }
  return {
    sessions: target.items.reduce((sum, item) => sum + Number(item.session_count || 0), 0),
    workspaces: target.items.length,
  };
}

function cleanSummary(result: ManagerCleanResult) {
  return `${result.success} cleaned, ${result.failed} failed, ${formatBytes(result.freed_bytes)} freed`;
}

function backupSummary(result: ManagerBackupResult) {
  return `${result.success} exported, ${result.failed} failed`;
}

function ProviderControls({
  providers,
  selected,
  onToggle,
}: {
  providers: ProviderInfo[];
  selected: string[];
  onToggle: (providerId: string) => void;
}) {
  if (!providers.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No providers</EmptyTitle>
          <EmptyDescription>No scan providers were returned by the backend.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollArea className="min-h-0 flex-1 pr-3" data-manager-provider-controls>
      <div className="flex flex-col gap-2">
        {providers.map((provider) => {
          const checked = selected.length === 0 || selected.includes(provider.id);
          return (
            <SelectableRowButton
              key={provider.id}
              selected={checked}
              title={provider.name}
              trailing={checked ? <CheckIcon className="text-muted-foreground size-4" aria-hidden /> : null}
              onClick={() => onToggle(provider.id)}
            />
          );
        })}
      </div>
    </ScrollArea>
  );
}

function ControlPanel({
  workspace,
  providers,
  selectedProviders,
  onToggleProvider,
}: {
  workspace: string | null | undefined;
  providers: ProviderInfo[];
  selectedProviders: string[];
  onToggleProvider: (providerId: string) => void;
}) {
  return (
    <PanelCard className="min-h-0" data-manager-control-panel>
      <section className="flex flex-col gap-3 border-b pb-4" data-manager-workspace-summary>
        <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
      </section>
      <ProviderControls providers={providers} selected={selectedProviders} onToggle={onToggleProvider} />
    </PanelCard>
  );
}

function StatsStrip({
  view,
  onViewChange,
  stats,
  providerCount,
  loading,
}: {
  view: ManagerView;
  onViewChange: (view: ManagerView) => void;
  stats: ReturnType<typeof useManagerStats>["data"];
  providerCount: number;
  loading: boolean;
}) {
  const placeholder = loading ? <Skeleton className="h-5 w-20" /> : "-";
  return (
    <MetricGrid columns="four" data-manager-view-tabs>
      <MetricTile
        active={view === "sessions"}
        label="Current Workspace"
        value={stats ? formatBytes(stats.current_workspace_size_bytes) : placeholder}
        hint={`${stats?.current_workspace_session_count ?? 0} sessions`}
        onClick={() => onViewChange("sessions")}
        variant="compact"
      />
      <MetricTile
        active={view === "workspaces"}
        label="All Workspaces"
        value={stats ? stats.all_workspace_count : placeholder}
        hint={`${stats?.all_workspace_session_count ?? 0} sessions`}
        onClick={() => onViewChange("workspaces")}
        variant="compact"
      />
      <MetricTile label="Selected Agents" value={providerCount || placeholder} hint="scan providers" variant="compact" />
      <MetricTile label="All Size" value={stats ? formatBytes(stats.all_workspace_size_bytes) : placeholder} hint="indexed storage" variant="compact" />
    </MetricGrid>
  );
}

function PreviewHeader({
  title,
  summary,
  search,
  searchPlaceholder,
  canAct,
  canSelect,
  canSelectFiltered,
  onSearchChange,
  onClean,
  onBackup,
  onCopyPaths,
  onSelectAll,
  onDeselectAll,
  onInvertSelection,
  onSelectFiltered,
}: {
  title: string;
  summary: string;
  search: string;
  searchPlaceholder: string;
  canAct: boolean;
  canSelect: boolean;
  canSelectFiltered: boolean;
  onSearchChange: (value: string) => void;
  onClean: () => void;
  onBackup: () => void;
  onCopyPaths: () => void;
  onSelectAll: () => void;
  onDeselectAll: () => void;
  onInvertSelection: () => void;
  onSelectFiltered: () => void;
}) {
  return (
    <div
      className="grid grid-cols-1 gap-3 border-b pb-2 md:grid-cols-[minmax(0,auto)_auto_minmax(0,1fr)] md:grid-rows-[auto_auto] md:items-center md:gap-x-4 md:gap-y-2"
      data-manager-preview-header
    >
      <div className="flex min-w-0 flex-col gap-1 md:row-span-2">
        <strong className="text-sm font-medium">{title}</strong>
        <span className="text-muted-foreground text-sm">{summary}</span>
      </div>

      <Separator orientation="vertical" className="hidden md:row-span-2 md:block md:h-auto md:self-stretch" />

      <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-manager-preview-actions>
        <Button type="button" variant="outline" disabled={!canAct} onClick={onClean} data-manager-action-clean>
          <Trash2Icon data-icon="inline-start" />
          Clean
        </Button>
        <Button type="button" variant="outline" disabled={!canAct} onClick={onBackup} data-manager-action-backup>
          <CopyIcon data-icon="inline-start" />
          Backup
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="outline" disabled={!canAct} data-manager-action-more>
              <MoreHorizontalIcon data-icon="inline-start" />
              More
              <ChevronDownIcon data-icon="inline-end" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem disabled={!canAct} onSelect={onCopyPaths}>
              <CopyIcon />
              Copy Paths
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="outline" disabled={!canSelect} data-manager-selection-menu>
              <CheckIcon data-icon="inline-start" />
              Selection
              <ChevronDownIcon data-icon="inline-end" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onSelectAll}>Select All</DropdownMenuItem>
            <DropdownMenuItem onSelect={onDeselectAll}>Deselect All</DropdownMenuItem>
            <DropdownMenuItem onSelect={onInvertSelection}>Invert Selection</DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled={!canSelectFiltered} onSelect={onSelectFiltered}>
              Select Filtered
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <div className="flex min-w-0 items-center justify-start gap-2 md:justify-end">
        <div className="relative w-52 shrink-0">
          <SearchIcon className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            className="pl-8"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={searchPlaceholder}
            data-manager-preview-search
          />
        </div>
        <Button type="button" variant="outline" disabled data-manager-preview-filters>
          <FilterIcon data-icon="inline-start" />
          Filters
        </Button>
      </div>
    </div>
  );
}

function RowShell({
  selected,
  onToggle,
  children,
}: {
  selected: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <article
      className={`relative cursor-pointer rounded-md border px-3 py-3${selected ? " bg-muted" : ""}`}
      data-selected={selected ? "true" : "false"}
      data-manager-row
      aria-selected={selected}
      onClick={onToggle}
    >
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">{children}</div>
      {selected ? (
        <CheckIcon className="text-muted-foreground pointer-events-none absolute right-3 bottom-3 size-4" aria-hidden />
      ) : null}
    </article>
  );
}

function SessionRows({
  items,
  selected,
  onToggle,
  onCleanRow,
  onBackupRow,
}: {
  items: ManagerItem[];
  selected: Set<string>;
  onToggle: (key: string) => void;
  onCleanRow: (item: ManagerItem) => void;
  onBackupRow: (item: ManagerItem) => void;
}) {
  if (!items.length) return <PageEmpty title="No sessions" description="No provider sessions matched the current manager filter." />;

  return (
    <div className="flex flex-col gap-2" data-manager-session-preview>
      {items.map((item) => {
        const key = sessionKey(item);
        const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
        return (
          <RowShell key={key} selected={selected.has(key)} onToggle={() => onToggle(key)}>
            <div className="flex min-w-0 flex-col gap-2">
              <Link to={href} className="truncate text-sm font-medium hover:underline" onClick={(event) => event.stopPropagation()}>
                {item.title || item.session_id}
              </Link>
              <div className="text-muted-foreground flex flex-wrap gap-2 text-xs">
                <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
                <span>{formatBytes(item.size_bytes)}</span>
                <span>Updated {formatDateTime(item.last_active_at)}</span>
              </div>
              <PathText value={item.project_dir || item.source_path} wrap="all" />
            </div>
            <div
              className="flex flex-wrap justify-start gap-2 md:justify-end"
              data-manager-row-actions
              onClick={(event) => event.stopPropagation()}
            >
              <Button asChild variant="outline">
                <Link to={href}>
                  View
                  <ArrowRightIcon data-icon="inline-end" />
                </Link>
              </Button>
              <Button type="button" variant="outline" onClick={() => onCleanRow(item)}>
                <Trash2Icon data-icon="inline-start" />
                Remove
              </Button>
              <Button type="button" variant="outline" onClick={() => onBackupRow(item)}>
                <MoreHorizontalIcon data-icon="inline-start" />
                More
              </Button>
            </div>
          </RowShell>
        );
      })}
    </div>
  );
}

function WorkspaceRows({
  items,
  selected,
  onToggle,
  onCleanRow,
  onBackupRow,
}: {
  items: ManagerWorkspaceItem[];
  selected: Set<string>;
  onToggle: (key: string) => void;
  onCleanRow: (item: ManagerWorkspaceItem) => void;
  onBackupRow: (item: ManagerWorkspaceItem) => void;
}) {
  if (!items.length) return <PageEmpty title="No workspaces" description="No workspace groups matched the current manager filter." />;

  return (
    <div className="flex flex-col gap-2" data-manager-workspace-preview>
      {items.map((item) => {
        const key = workspaceKey(item);
        const href = `/manager?view=sessions&provider=${encodeURIComponent(item.provider_id)}&workspace=${encodeURIComponent(item.workspace)}`;
        return (
          <RowShell key={key} selected={selected.has(key)} onToggle={() => onToggle(key)}>
            <div className="flex min-w-0 flex-col gap-2">
              <Link to={href} className="truncate text-sm font-medium hover:underline" onClick={(event) => event.stopPropagation()}>
                {workspaceName(item.workspace)}
              </Link>
              <div className="text-muted-foreground flex flex-wrap gap-2 text-xs">
                <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
                <span>{item.session_count} sessions</span>
                <span>{formatBytes(item.total_size_bytes)}</span>
                <span>Updated {formatDateTime(item.last_active_at)}</span>
              </div>
              <span className="text-muted-foreground break-words font-mono text-xs">{item.workspace}</span>
            </div>
            <div
              className="flex flex-wrap justify-start gap-2 md:justify-end"
              data-manager-row-actions
              onClick={(event) => event.stopPropagation()}
            >
              <Button asChild variant="outline">
                <Link to={href}>
                  View
                  <ArrowRightIcon data-icon="inline-end" />
                </Link>
              </Button>
              <Button type="button" variant="outline" onClick={() => onCleanRow(item)}>
                <Trash2Icon data-icon="inline-start" />
                Remove
              </Button>
              <Button type="button" variant="outline" onClick={() => onBackupRow(item)}>
                <MoreHorizontalIcon data-icon="inline-start" />
                More
              </Button>
            </div>
          </RowShell>
        );
      })}
    </div>
  );
}

function ResultPanel({
  view,
  setView,
  stats,
  statsLoading,
  providerCount,
  sessions,
  workspaces,
  search,
  onSearchChange,
  selectedSessions,
  selectedWorkspaces,
  selectedSessionBytes,
  selectedWorkspaceBytes,
  onToggleSession,
  onToggleWorkspace,
  onSelectAllSessions,
  onDeselectAllSessions,
  onInvertSessions,
  onSelectFilteredSessions,
  onSelectAllWorkspaces,
  onDeselectAllWorkspaces,
  onInvertWorkspaces,
  onSelectFilteredWorkspaces,
  onCopySessionPaths,
  onCopyWorkspacePaths,
  onOpenAction,
}: {
  view: ManagerView;
  setView: (view: ManagerView) => void;
  stats: ReturnType<typeof useManagerStats>;
  statsLoading: boolean;
  providerCount: number;
  sessions: ReturnType<typeof useManagerPreview>;
  workspaces: ReturnType<typeof useManagerWorkspaces>;
  search: string;
  onSearchChange: (value: string) => void;
  selectedSessions: Set<string>;
  selectedWorkspaces: Set<string>;
  selectedSessionBytes: number;
  selectedWorkspaceBytes: number;
  onToggleSession: (key: string) => void;
  onToggleWorkspace: (key: string) => void;
  onSelectAllSessions: () => void;
  onDeselectAllSessions: () => void;
  onInvertSessions: () => void;
  onSelectFilteredSessions: () => void;
  onSelectAllWorkspaces: () => void;
  onDeselectAllWorkspaces: () => void;
  onInvertWorkspaces: () => void;
  onSelectFilteredWorkspaces: () => void;
  onCopySessionPaths: () => void;
  onCopyWorkspacePaths: () => void;
  onOpenAction: (target: ManagerActionTarget) => void;
}) {
  const sessionRows = sessions.data?.items ?? [];
  const workspaceRows = workspaces.data?.items ?? [];
  const filteredSessionRows = sessionRows.filter((item) => matchesSessionSearch(item, search));
  const filteredWorkspaceRows = workspaceRows.filter((item) => matchesWorkspaceSearch(item, search));
  const searchActive = search.trim().length > 0;

  return (
    <PanelCard variant="plain" className="grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4" data-manager-result-panel>
        <StatsStrip view={view} onViewChange={setView} stats={stats.data} providerCount={providerCount} loading={statsLoading} />
        <Separator />
        <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3">
          {view === "sessions" ? (
            <>
              <PreviewHeader
                title="Manager Preview"
                summary={selectionSummary(
                  sessions.data?.total_count,
                  filteredSessionRows.length,
                  selectedSessions.size,
                  selectedSessionBytes,
                  searchActive,
                )}
                search={search}
                searchPlaceholder="Search title, id, provider, or path"
                canSelect={sessionRows.length > 0}
                canSelectFiltered={searchActive && filteredSessionRows.length > 0}
                canAct={selectedSessions.size > 0}
                onSearchChange={onSearchChange}
                onClean={() => onOpenAction({ kind: "clean-sessions", items: sessionRows.filter((item) => selectedSessions.has(sessionKey(item))) })}
                onBackup={() => onOpenAction({ kind: "backup-sessions", items: sessionRows.filter((item) => selectedSessions.has(sessionKey(item))) })}
                onCopyPaths={onCopySessionPaths}
                onSelectAll={onSelectAllSessions}
                onDeselectAll={onDeselectAllSessions}
                onInvertSelection={onInvertSessions}
                onSelectFiltered={onSelectFilteredSessions}
              />
              <ScrollArea className="min-h-0 pr-3">
                {sessions.isLoading ? (
                  <PageSkeleton />
                ) : (
                  <SessionRows
                    items={filteredSessionRows}
                    selected={selectedSessions}
                    onToggle={onToggleSession}
                    onCleanRow={(item) => onOpenAction({ kind: "clean-sessions", items: [item] })}
                    onBackupRow={(item) => onOpenAction({ kind: "backup-sessions", items: [item] })}
                  />
                )}
              </ScrollArea>
            </>
          ) : (
            <>
              <PreviewHeader
                title="Workspace Preview"
                summary={selectionSummary(
                  workspaces.data?.total_count,
                  filteredWorkspaceRows.length,
                  selectedWorkspaces.size,
                  selectedWorkspaceBytes,
                  searchActive,
                )}
                search={search}
                searchPlaceholder="Search workspace, provider, or path"
                canSelect={workspaceRows.length > 0}
                canSelectFiltered={searchActive && filteredWorkspaceRows.length > 0}
                canAct={selectedWorkspaces.size > 0}
                onSearchChange={onSearchChange}
                onClean={() =>
                  onOpenAction({ kind: "clean-workspaces", items: workspaceRows.filter((item) => selectedWorkspaces.has(workspaceKey(item))) })
                }
                onBackup={() =>
                  onOpenAction({ kind: "backup-workspaces", items: workspaceRows.filter((item) => selectedWorkspaces.has(workspaceKey(item))) })
                }
                onCopyPaths={onCopyWorkspacePaths}
                onSelectAll={onSelectAllWorkspaces}
                onDeselectAll={onDeselectAllWorkspaces}
                onInvertSelection={onInvertWorkspaces}
                onSelectFiltered={onSelectFilteredWorkspaces}
              />
              <ScrollArea className="min-h-0 pr-3">
                {workspaces.isLoading ? (
                  <PageSkeleton />
                ) : (
                  <WorkspaceRows
                    items={filteredWorkspaceRows}
                    selected={selectedWorkspaces}
                    onToggle={onToggleWorkspace}
                    onCleanRow={(item) => onOpenAction({ kind: "clean-workspaces", items: [item] })}
                    onBackupRow={(item) => onOpenAction({ kind: "backup-workspaces", items: [item] })}
                  />
                )}
              </ScrollArea>
            </>
          )}
        </div>
    </PanelCard>
  );
}

export function ManagerPage() {
  const [searchParams] = useSearchParams();
  const initialProvider = searchParams.get("provider");
  const initialView: ManagerView = searchParams.get("view") === "workspaces" ? "workspaces" : "sessions";
  const [view, setView] = useState<ManagerView>(initialView);
  const [search, setSearch] = useState("");
  const [selectedProviders, setSelectedProviders] = useState<string[]>(() => (initialProvider ? [initialProvider] : []));
  const [selectedSessions, setSelectedSessions] = useState<Set<string>>(() => new Set());
  const [selectedWorkspaces, setSelectedWorkspaces] = useState<Set<string>>(() => new Set());
  const [actionTarget, setActionTarget] = useState<ManagerActionTarget | null>(null);
  const [actionReport, setActionReport] = useState<ManagerActionReport | null>(null);
  const queryClient = useQueryClient();

  const filter = useMemo<ManagerFilter>(
    () => ({ providers: selectedProviders, sort: "recent", limit: 100 }),
    [selectedProviders],
  );

  const meta = useManagerMeta();
  const providers = useManagerProviders();
  const stats = useManagerStats(filter);
  const sessions = useManagerPreview(filter);
  const workspaces = useManagerWorkspaces(filter);

  const managerAction = useMutation({
    mutationFn: async (target: ManagerActionTarget): Promise<ManagerActionReport> => {
      const outputDir = meta.data?.settings.default_backup_dir || "./backups";

      if (target.kind === "clean-sessions") {
        const result = await cleanManagerItems({ items: target.items });
        return { title: "Clean Selected", summary: cleanSummary(result), lines: result.errors || [] };
      }

      if (target.kind === "backup-sessions") {
        const result = await backupManagerItems({ items: target.items, output_dir: outputDir });
        return { title: "Backup Selected", summary: backupSummary(result), lines: [...(result.files || []), ...(result.errors || [])] };
      }

      if (target.kind === "clean-workspaces") {
        let success = 0;
        let failed = 0;
        let freed = 0;
        const lines: string[] = [];
        const errors: string[] = [];

        for (const item of target.items) {
          const result = await cleanManagerWorkspace({ provider_id: item.provider_id, workspace: item.workspace });
          success += result.success;
          failed += result.failed;
          freed += result.freed_bytes;
          lines.push(`${item.provider_id} / ${workspaceName(item.workspace)}: ${result.success}/${result.failed}`);
          errors.push(...(result.errors || []));
        }

        return {
          title: "Clean Workspace Selected",
          summary: cleanSummary({ success, failed, freed_bytes: freed, errors }),
          lines: [...lines, ...errors],
        };
      }

      let success = 0;
      let failed = 0;
      const lines: string[] = [];
      const files: string[] = [];
      const errors: string[] = [];

      for (const item of target.items) {
        const result = await backupManagerWorkspace({ provider_id: item.provider_id, workspace: item.workspace, output_dir: outputDir });
        success += result.success;
        failed += result.failed;
        lines.push(`${item.provider_id} / ${workspaceName(item.workspace)}: ${result.success}/${result.failed}`);
        files.push(...(result.files || []));
        errors.push(...(result.errors || []));
      }

      return {
        title: "Backup Workspace Selected",
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
      toast.error("Manager action failed", { description: error instanceof Error ? error.message : String(error) });
    },
  });

  if (providers.isLoading || meta.isLoading) return <PageSkeleton />;
  if (providers.error) return <PageError title="Manager providers failed to load" message={providers.error.message} />;
  if (meta.error) return <PageError title="Manager workspace failed to load" message={meta.error.message} />;
  if (stats.error) return <PageError title="Manager stats failed to load" message={stats.error.message} />;
  if (sessions.error) return <PageError title="Manager sessions failed to load" message={sessions.error.message} />;
  if (workspaces.error) return <PageError title="Manager workspaces failed to load" message={workspaces.error.message} />;

  const options = providerOptions(providers.data);
  const sessionRows = sessions.data?.items ?? [];
  const workspaceRows = workspaces.data?.items ?? [];
  const selectedSessionBytes = sessionRows
    .filter((item) => selectedSessions.has(sessionKey(item)))
    .reduce((sum, item) => sum + item.size_bytes, 0);
  const selectedWorkspaceBytes = workspaceRows
    .filter((item) => selectedWorkspaces.has(workspaceKey(item)))
    .reduce((sum, item) => sum + item.total_size_bytes, 0);
  const providerCount = stats.data?.selected_agent_count ?? (selectedProviders.length || options.length);

  function toggleProvider(providerId: string) {
    setSelectedProviders((current) =>
      current.includes(providerId) ? current.filter((id) => id !== providerId) : [...current, providerId],
    );
    setSearch("");
    setSelectedSessions(new Set());
    setSelectedWorkspaces(new Set());
  }

  function toggleSession(key: string) {
    setSelectedSessions((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function toggleWorkspace(key: string) {
    setSelectedWorkspaces((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function selectAllSessions() {
    setSelectedSessions(new Set(sessionRows.map(sessionKey)));
  }

  function deselectAllSessions() {
    setSelectedSessions(new Set());
  }

  function invertSessions() {
    setSelectedSessions((current) => {
      const next = new Set(current);
      for (const item of sessionRows) {
        const key = sessionKey(item);
        if (next.has(key)) next.delete(key);
        else next.add(key);
      }
      return next;
    });
  }

  function selectFilteredSessions() {
    setSelectedSessions(
      new Set(sessionRows.filter((item) => matchesSessionSearch(item, search)).map(sessionKey)),
    );
  }

  function selectAllWorkspaces() {
    setSelectedWorkspaces(new Set(workspaceRows.map(workspaceKey)));
  }

  function deselectAllWorkspaces() {
    setSelectedWorkspaces(new Set());
  }

  function invertWorkspaces() {
    setSelectedWorkspaces((current) => {
      const next = new Set(current);
      for (const item of workspaceRows) {
        const key = workspaceKey(item);
        if (next.has(key)) next.delete(key);
        else next.add(key);
      }
      return next;
    });
  }

  function selectFilteredWorkspaces() {
    setSelectedWorkspaces(
      new Set(workspaceRows.filter((item) => matchesWorkspaceSearch(item, search)).map(workspaceKey)),
    );
  }

  async function copySessionPaths() {
    const paths = sessionRows
      .filter((item) => selectedSessions.has(sessionKey(item)))
      .map((item) => item.project_dir || item.source_path)
      .filter(Boolean);

    if (!paths.length) {
      toast.error("No paths to copy");
      return;
    }

    try {
      await navigator.clipboard.writeText(paths.join("\n"));
      toast.success(`Copied ${paths.length} path${paths.length === 1 ? "" : "s"}`);
    } catch (error) {
      toast.error("Copy failed", { description: error instanceof Error ? error.message : String(error) });
    }
  }

  async function copyWorkspacePaths() {
    const paths = workspaceRows
      .filter((item) => selectedWorkspaces.has(workspaceKey(item)))
      .map((item) => item.workspace)
      .filter(Boolean);

    if (!paths.length) {
      toast.error("No paths to copy");
      return;
    }

    try {
      await navigator.clipboard.writeText(paths.join("\n"));
      toast.success(`Copied ${paths.length} path${paths.length === 1 ? "" : "s"}`);
    } catch (error) {
      toast.error("Copy failed", { description: error instanceof Error ? error.message : String(error) });
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
      <TwoPanePage data-manager-page-layout>
        <ControlPanel
          workspace={meta.data?.selected_workspace}
          providers={options}
          selectedProviders={selectedProviders}
          onToggleProvider={toggleProvider}
        />
        <ResultPanel
          view={view}
          setView={(nextView) => {
            if (nextView !== view) setSearch("");
            setView(nextView);
          }}
          stats={stats}
          statsLoading={stats.isLoading}
          providerCount={providerCount}
          sessions={sessions}
          workspaces={workspaces}
          search={search}
          onSearchChange={setSearch}
          selectedSessions={selectedSessions}
          selectedWorkspaces={selectedWorkspaces}
          selectedSessionBytes={selectedSessionBytes}
          selectedWorkspaceBytes={selectedWorkspaceBytes}
          onToggleSession={toggleSession}
          onToggleWorkspace={toggleWorkspace}
          onSelectAllSessions={selectAllSessions}
          onDeselectAllSessions={deselectAllSessions}
          onInvertSessions={invertSessions}
          onSelectFilteredSessions={selectFilteredSessions}
          onSelectAllWorkspaces={selectAllWorkspaces}
          onDeselectAllWorkspaces={deselectAllWorkspaces}
          onInvertWorkspaces={invertWorkspaces}
          onSelectFilteredWorkspaces={selectFilteredWorkspaces}
          onCopySessionPaths={copySessionPaths}
          onCopyWorkspacePaths={copyWorkspacePaths}
          onOpenAction={openAction}
        />
      </TwoPanePage>

      <Dialog open={Boolean(actionTarget)} onOpenChange={(open) => !open && setActionTarget(null)}>
        <DialogContent data-manager-action-dialog data-manager-clean-dialog={isCleanAction(actionTarget) ? "true" : undefined} data-manager-backup-dialog={isBackupAction(actionTarget) ? "true" : undefined}>
          <DialogHeader>
            <DialogTitle>{actionTitle(actionTarget)}</DialogTitle>
            <DialogDescription>
              {isCleanAction(actionTarget)
                ? "Confirm removal of the selected manager targets."
                : "Confirm backup of the selected manager targets."}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2 rounded-md border p-3 font-mono text-xs">
            {pendingStats.workspaces > 0 ? <span>Workspaces: {pendingStats.workspaces}</span> : null}
            <span>Sessions: {pendingStats.sessions}</span>
            {isBackupAction(actionTarget) ? <span>Backup Dir: {backupDir}</span> : null}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setActionTarget(null)} disabled={managerAction.isPending}>
              Cancel
            </Button>
            <Button
              type="button"
              variant={isCleanAction(actionTarget) ? "destructive" : "default"}
              onClick={() => actionTarget && managerAction.mutate(actionTarget)}
              disabled={!actionTarget || managerAction.isPending}
            >
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(actionReport)} onOpenChange={(open) => !open && setActionReport(null)}>
        <DialogContent data-manager-action-result>
          <DialogHeader>
            <DialogTitle>{actionReport?.title}</DialogTitle>
            <DialogDescription>{actionReport?.summary}</DialogDescription>
          </DialogHeader>
          {actionReport?.lines.length ? (
            <ScrollArea className="max-h-72 rounded-md border p-3">
              <div className="flex flex-col gap-2 font-mono text-xs">
                {actionReport.lines.map((line, index) => (
                  <span key={`${line}-${index}`} className="break-words">
                    {line}
                  </span>
                ))}
              </div>
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
