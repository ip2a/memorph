import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { ArrowRightIcon, CheckIcon, CopyIcon, FilterIcon, MoreHorizontalIcon, Trash2Icon } from "lucide-react";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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
import { compactPath, formatBytes, formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
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

function workspaceName(path: string | null | undefined) {
  if (!path) return "No workspace";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path;
}

function providerOptions(providers: ProviderInfo[] | undefined) {
  return (providers ?? []).filter((provider) => provider.scan);
}

function selectionSummary(total: number | undefined, selected: number, bytes: number) {
  return `${total ?? 0} total / ${selected} selected / ${formatBytes(bytes)} selected`;
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

function WorkspaceSummary({ workspace }: { workspace: string | null | undefined }) {
  return (
    <section className="flex flex-col gap-3 border-b pb-4" data-manager-workspace-summary>
      <div className="flex flex-col gap-1">
        <span className="text-muted-foreground font-mono text-xs uppercase">Workspace</span>
        <strong className="text-lg font-semibold leading-tight">{workspaceName(workspace)}</strong>
        <span className="text-muted-foreground break-words font-mono text-xs">{workspace || "-"}</span>
      </div>
    </section>
  );
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
    <section className="flex min-h-0 flex-col gap-3" data-manager-provider-controls>
      <div className="flex items-center justify-between gap-3">
        <strong className="text-sm font-medium">Providers</strong>
        <Badge variant="outline">{selected.length || providers.length}</Badge>
      </div>
      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="flex flex-col gap-2">
          {providers.map((provider) => {
            const checked = selected.length === 0 || selected.includes(provider.id);
            const explicitlySelected = selected.includes(provider.id);
            return (
              <Button
                key={provider.id}
                type="button"
                variant={checked ? "secondary" : "outline"}
                className="h-auto min-h-11 justify-start px-3 py-2 text-left"
                onClick={() => onToggle(provider.id)}
              >
                <span className="grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3">
                  <Checkbox checked={checked} tabIndex={-1} aria-hidden />
                  <span className="flex min-w-0 flex-col gap-1">
                    <strong className="truncate text-sm font-medium">{provider.name}</strong>
                    <span className="text-muted-foreground truncate font-mono text-xs">{provider.id}</span>
                  </span>
                  <Badge variant={explicitlySelected || selected.length === 0 ? "secondary" : "outline"}>{checked ? "on" : "off"}</Badge>
                </span>
              </Button>
            );
          })}
        </div>
      </ScrollArea>
    </section>
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
    <Card className="min-h-0" data-manager-control-panel>
      <CardContent className="flex h-full min-h-0 flex-col gap-4">
        <WorkspaceSummary workspace={workspace} />
        <ProviderControls providers={providers} selected={selectedProviders} onToggle={onToggleProvider} />
      </CardContent>
    </Card>
  );
}

function StatTile({
  active,
  readonly,
  label,
  value,
  hint,
  onClick,
}: {
  active?: boolean;
  readonly?: boolean;
  label: string;
  value: ReactNode;
  hint: string;
  onClick?: () => void;
}) {
  const content = (
    <span className="flex min-w-0 flex-col items-start gap-1 text-left">
      <span className="text-muted-foreground truncate text-xs">{label}</span>
      <strong className="truncate text-base font-semibold">{value}</strong>
      <span className="text-muted-foreground truncate font-mono text-xs">{hint}</span>
    </span>
  );

  if (readonly) {
    return <div className="rounded-md border px-3 py-2">{content}</div>;
  }

  return (
    <Button
      type="button"
      variant={active ? "secondary" : "outline"}
      className="h-auto justify-start px-3 py-2"
      onClick={onClick}
    >
      {content}
    </Button>
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
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" data-manager-view-tabs>
      <StatTile
        active={view === "sessions"}
        label="Current Workspace"
        value={stats ? formatBytes(stats.current_workspace_size_bytes) : placeholder}
        hint={`${stats?.current_workspace_session_count ?? 0} sessions`}
        onClick={() => onViewChange("sessions")}
      />
      <StatTile
        active={view === "workspaces"}
        label="All Workspaces"
        value={stats ? stats.all_workspace_count : placeholder}
        hint={`${stats?.all_workspace_session_count ?? 0} sessions`}
        onClick={() => onViewChange("workspaces")}
      />
      <StatTile readonly label="Selected Agents" value={providerCount || placeholder} hint="scan providers" />
      <StatTile readonly label="All Size" value={stats ? formatBytes(stats.all_workspace_size_bytes) : placeholder} hint="indexed storage" />
    </div>
  );
}

function PreviewHeader({
  title,
  summary,
  canSelectAll,
  canAct,
  allSelected,
  onClean,
  onBackup,
  onSelectAll,
}: {
  title: string;
  summary: string;
  canSelectAll: boolean;
  canAct: boolean;
  allSelected: boolean;
  onClean: () => void;
  onBackup: () => void;
  onSelectAll: () => void;
}) {
  return (
    <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start" data-manager-preview-header>
      <div className="flex min-w-0 flex-col gap-1">
        <strong className="text-sm font-medium">{title}</strong>
        <span className="text-muted-foreground text-sm">{summary}</span>
      </div>
      <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-manager-preview-actions>
        <Button type="button" variant="outline" size="sm" disabled>
          <FilterIcon data-icon="inline-start" />
          Filters
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={!canAct} onClick={onClean}>
          <Trash2Icon data-icon="inline-start" />
          Clean Selected
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={!canAct} onClick={onBackup}>
          <CopyIcon data-icon="inline-start" />
          Backup Selected
        </Button>
        <Button type="button" variant={allSelected ? "secondary" : "outline"} size="sm" disabled={!canSelectAll} onClick={onSelectAll}>
          <CheckIcon data-icon="inline-start" />
          {allSelected ? "Deselect All" : "Select All"}
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
      className={cn("rounded-md border px-3 py-3", selected && "bg-muted")}
      data-manager-row
      data-selected={selected ? "true" : "false"}
    >
      <div className="grid gap-3 md:grid-cols-[auto_minmax(0,1fr)_auto] md:items-start">
        <Checkbox checked={selected} onCheckedChange={onToggle} aria-label="Select row" />
        {children}
      </div>
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
              <Link to={href} className="truncate text-sm font-medium hover:underline">
                {item.title || item.session_id}
              </Link>
              <div className="text-muted-foreground flex flex-wrap gap-2 text-xs">
                <Badge variant="outline">{item.provider_name || item.provider_id}</Badge>
                <span>{formatBytes(item.size_bytes)}</span>
                <span>Updated {formatDateTime(item.last_active_at)}</span>
              </div>
              <span className="text-muted-foreground truncate font-mono text-xs">{compactPath(item.project_dir || item.source_path)}</span>
            </div>
            <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-manager-row-actions>
              <Button asChild variant="outline" size="sm">
                <Link to={href}>
                  View
                  <ArrowRightIcon data-icon="inline-end" />
                </Link>
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => onCleanRow(item)}>
                <Trash2Icon data-icon="inline-start" />
                Remove
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => onBackupRow(item)}>
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
              <Link to={href} className="truncate text-sm font-medium hover:underline">
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
            <div className="flex flex-wrap justify-start gap-2 md:justify-end" data-manager-row-actions>
              <Button asChild variant="outline" size="sm">
                <Link to={href}>
                  View
                  <ArrowRightIcon data-icon="inline-end" />
                </Link>
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => onCleanRow(item)}>
                <Trash2Icon data-icon="inline-start" />
                Remove
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => onBackupRow(item)}>
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
  selectedSessions,
  selectedWorkspaces,
  selectedSessionBytes,
  selectedWorkspaceBytes,
  onToggleSession,
  onToggleWorkspace,
  onToggleAllSessions,
  onToggleAllWorkspaces,
  onOpenAction,
}: {
  view: ManagerView;
  setView: (view: ManagerView) => void;
  stats: ReturnType<typeof useManagerStats>;
  statsLoading: boolean;
  providerCount: number;
  sessions: ReturnType<typeof useManagerPreview>;
  workspaces: ReturnType<typeof useManagerWorkspaces>;
  selectedSessions: Set<string>;
  selectedWorkspaces: Set<string>;
  selectedSessionBytes: number;
  selectedWorkspaceBytes: number;
  onToggleSession: (key: string) => void;
  onToggleWorkspace: (key: string) => void;
  onToggleAllSessions: () => void;
  onToggleAllWorkspaces: () => void;
  onOpenAction: (target: ManagerActionTarget) => void;
}) {
  const sessionRows = sessions.data?.items ?? [];
  const workspaceRows = workspaces.data?.items ?? [];
  const allSessionsSelected = sessionRows.length > 0 && selectedSessions.size === sessionRows.length;
  const allWorkspacesSelected = workspaceRows.length > 0 && selectedWorkspaces.size === workspaceRows.length;

  return (
    <Card className="min-h-0" data-manager-result-panel>
      <CardContent className="grid h-full min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4">
        <StatsStrip view={view} onViewChange={setView} stats={stats.data} providerCount={providerCount} loading={statsLoading} />
        <Separator />
        <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-3">
          {view === "sessions" ? (
            <>
              <PreviewHeader
                title="Manager Preview"
                summary={selectionSummary(sessions.data?.total_count, selectedSessions.size, selectedSessionBytes)}
                canSelectAll={sessionRows.length > 0}
                canAct={selectedSessions.size > 0}
                allSelected={allSessionsSelected}
                onClean={() => onOpenAction({ kind: "clean-sessions", items: sessionRows.filter((item) => selectedSessions.has(sessionKey(item))) })}
                onBackup={() => onOpenAction({ kind: "backup-sessions", items: sessionRows.filter((item) => selectedSessions.has(sessionKey(item))) })}
                onSelectAll={onToggleAllSessions}
              />
              <ScrollArea className="min-h-0 pr-3">
                {sessions.isLoading ? (
                  <PageSkeleton />
                ) : (
                  <SessionRows
                    items={sessionRows}
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
                summary={selectionSummary(workspaces.data?.total_count, selectedWorkspaces.size, selectedWorkspaceBytes)}
                canSelectAll={workspaceRows.length > 0}
                canAct={selectedWorkspaces.size > 0}
                allSelected={allWorkspacesSelected}
                onClean={() =>
                  onOpenAction({ kind: "clean-workspaces", items: workspaceRows.filter((item) => selectedWorkspaces.has(workspaceKey(item))) })
                }
                onBackup={() =>
                  onOpenAction({ kind: "backup-workspaces", items: workspaceRows.filter((item) => selectedWorkspaces.has(workspaceKey(item))) })
                }
                onSelectAll={onToggleAllWorkspaces}
              />
              <ScrollArea className="min-h-0 pr-3">
                {workspaces.isLoading ? (
                  <PageSkeleton />
                ) : (
                  <WorkspaceRows
                    items={workspaceRows}
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
      </CardContent>
    </Card>
  );
}

export function ManagerPage() {
  const [view, setView] = useState<ManagerView>("sessions");
  const [selectedProviders, setSelectedProviders] = useState<string[]>([]);
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

  function toggleAllSessions() {
    setSelectedSessions((current) => {
      if (current.size === sessionRows.length) return new Set();
      return new Set(sessionRows.map(sessionKey));
    });
  }

  function toggleAllWorkspaces() {
    setSelectedWorkspaces((current) => {
      if (current.size === workspaceRows.length) return new Set();
      return new Set(workspaceRows.map(workspaceKey));
    });
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
      <div className="grid min-h-[calc(100vh-124px)] grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,0.44fr)_minmax(0,1fr)]" data-manager-page-layout>
        <ControlPanel
          workspace={meta.data?.selected_workspace}
          providers={options}
          selectedProviders={selectedProviders}
          onToggleProvider={toggleProvider}
        />
        <ResultPanel
          view={view}
          setView={setView}
          stats={stats}
          statsLoading={stats.isLoading}
          providerCount={providerCount}
          sessions={sessions}
          workspaces={workspaces}
          selectedSessions={selectedSessions}
          selectedWorkspaces={selectedWorkspaces}
          selectedSessionBytes={selectedSessionBytes}
          selectedWorkspaceBytes={selectedWorkspaceBytes}
          onToggleSession={toggleSession}
          onToggleWorkspace={toggleWorkspace}
          onToggleAllSessions={toggleAllSessions}
          onToggleAllWorkspaces={toggleAllWorkspaces}
          onOpenAction={openAction}
        />
      </div>

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
