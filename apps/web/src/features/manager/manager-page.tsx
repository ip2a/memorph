import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArchiveIcon,
  ArrowRightIcon,
  CheckIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  SearchIcon,
  ShieldAlertIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react";
import {
  PageEmpty,
  PageError,
  PageSkeleton,
} from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
import { workspaceName } from "@/components/shared/workspace-name";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TrailingMoreButtonGroup } from "@/components/shared/trailing-more-button-group";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenuItem,
  DropdownMenuSeparator,
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
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "sonner";
import {
  backupManagerItems,
  backupManagerWorkspace,
  cleanManagerItems,
  cleanManagerWorkspace,
} from "@/lib/api";
import { formatBytes, formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { queryKeys } from "@/lib/query-keys";
import type {
  ManagerBackupResult,
  ManagerCleanResult,
  ManagerItem,
  ManagerWorkspaceItem,
} from "@/lib/types";
import {
  useManagerMeta,
  useManagerPreview,
  useManagerProviderCatalog,
  useManagerStats,
  useManagerWorkspaces,
} from "@/features/manager/queries";
import { managerProviderOptions } from "@/features/manager/manager-providers";
import type { ManagerProviderOption } from "@/features/manager/manager-providers";
import {
  readManagerRouteState,
  resetManagerPageParam,
  resolveManagerRequest,
} from "@/features/manager/manager-route-state";
import type {
  ManagerProviderSelection,
  ManagerScope,
  ManagerSort,
  ManagerView,
} from "@/features/manager/manager-route-state";
import {
  DEFAULT_MANAGER_PAGE_SIZE,
  managerTotalPages,
} from "@/features/manager/manager-pagination";
import type { ManagerPageSize } from "@/features/manager/manager-pagination";
import { ManagerResultPagination } from "@/features/manager/manager-result-pagination";
import { useUiStore } from "@/stores/ui-store";

const MANAGER_SEARCH_DEBOUNCE_MS = 300;

type ManagerActionTarget =
  | { kind: "delete-sessions"; items: ManagerItem[] }
  | { kind: "backup-sessions"; items: ManagerItem[] }
  | { kind: "delete-workspaces"; items: ManagerWorkspaceItem[] }
  | { kind: "backup-workspaces"; items: ManagerWorkspaceItem[] };

type ManagerActionOutcome = "success" | "partial" | "failure";

type ManagerActionReport = {
  title: string;
  summary: string;
  lines: string[];
  outcome: ManagerActionOutcome;
};

type ManagerDeleteTarget = Extract<
  ManagerActionTarget,
  { kind: `delete-${string}` }
>;
type ManagerBackupTarget = Extract<
  ManagerActionTarget,
  { kind: `backup-${string}` }
>;

function workspaceIdentity(item: ManagerWorkspaceItem) {
  return JSON.stringify([item.provider_id, item.workspace]);
}

type Translate = (
  key: I18nKey,
  vars?: Record<string, string | number | null | undefined>,
) => string;

function actionTitle(target: ManagerActionTarget | null, t: Translate) {
  switch (target?.kind) {
    case "delete-sessions":
      return t(target.items.length === 1 ? "managerDeleteSession" : "managerDeleteSessions");
    case "backup-sessions":
      return t(target.items.length === 1 ? "managerBackupSession" : "managerBackupSessions");
    case "delete-workspaces":
      return t("managerDeleteWorkspace");
    case "backup-workspaces":
      return t(target.items.length === 1 ? "managerBackupWorkspace" : "managerBackupWorkspaces");
    default:
      return t("managerAction");
  }
}

function isBackupAction(
  target: ManagerActionTarget | null,
): target is ManagerBackupTarget {
  return (
    target?.kind === "backup-sessions" || target?.kind === "backup-workspaces"
  );
}

function isDeleteAction(
  target: ManagerActionTarget | null,
): target is ManagerDeleteTarget {
  return (
    target?.kind === "delete-sessions" || target?.kind === "delete-workspaces"
  );
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

function actionOutcome(success: number, failed: number): ManagerActionOutcome {
  if (failed === 0) return "success";
  return success > 0 ? "partial" : "failure";
}

function actionResultTitle(
  outcome: ManagerActionOutcome,
  successTitle: string,
  partialTitle: string,
  failureTitle: string,
) {
  if (outcome === "success") return successTitle;
  return outcome === "partial" ? partialTitle : failureTitle;
}

function actionProviders(target: ManagerActionTarget | null) {
  if (!target) return [];
  return Array.from(
    new Set(target.items.map((item) => item.provider_name || item.provider_id)),
  );
}

function actionWorkspaces(target: ManagerActionTarget | null) {
  if (!target) return [];
  return Array.from(
    new Set(
      target.items
        .map((item) =>
          "workspace" in item ? item.workspace : item.project_dir,
        )
        .filter((workspace): workspace is string => Boolean(workspace)),
    ),
  );
}

function cleanSummary(result: ManagerCleanResult, t: Translate) {
  return t("managerDeletedSummary", { success: result.success, failed: result.failed, size: formatBytes(result.freed_bytes) });
}

function backupSummary(result: ManagerBackupResult, t: Translate) {
  return t("managerBackedUpSummary", { success: result.success, failed: result.failed });
}

function ActionTargetSummary({
  target,
  backupDir,
}: {
  target: ManagerActionTarget;
  backupDir?: string;
}) {
  const { t } = useI18n();
  const stats = actionStats(target);
  const providers = actionProviders(target);
  const workspaces = actionWorkspaces(target);
  const deleting = isDeleteAction(target);

  return (
    <div className="grid gap-3" data-manager-action-summary>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2 rounded-lg border bg-muted/30 p-3 text-xs">
        <dt className="text-muted-foreground">{t("managerSelectedObjects")}</dt>
        <dd className="text-right font-medium">{target.items.length}</dd>
        {stats.workspaces > 0 ? (
          <>
            <dt className="text-muted-foreground">{t("managerWorkspaces")}</dt>
            <dd className="text-right font-medium">{stats.workspaces}</dd>
          </>
        ) : null}
        <dt className="text-muted-foreground">{t("managerSessionsAffected")}</dt>
        <dd className="text-right font-medium">{stats.sessions}</dd>
        <dt className="text-muted-foreground">{t("managerEstimatedSize")}</dt>
        <dd className="text-right font-medium">{formatBytes(stats.bytes)}</dd>
        <dt className="text-muted-foreground">{t("managerProvider")}</dt>
        <dd className="break-words text-right font-medium">
          {providers.length ? providers.join(", ") : t("managerUnknown")}
        </dd>
        <dt className="text-muted-foreground">{t("managerWorkspace")}</dt>
        <dd className="min-w-0 whitespace-pre-wrap break-all text-right font-mono">
          {workspaces.length ? workspaces.join("\n") : t("managerUnknown")}
        </dd>
        {backupDir ? (
          <>
            <dt className="text-muted-foreground">{t("managerBackupDirectory")}</dt>
            <dd className="min-w-0 break-all text-right font-mono">
              {backupDir}
            </dd>
          </>
        ) : null}
      </dl>
      {deleting ? (
        <div className="flex gap-2 rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          <ShieldAlertIcon
            className="mt-0.5 size-4 shrink-0"
            aria-hidden="true"
          />
          <p>{t("managerDeleteWarning")}</p>
        </div>
      ) : null}
    </div>
  );
}

function ProviderFilter({
  providers,
  selected,
  selection,
  onToggle,
}: {
  providers: ManagerProviderOption[];
  selected: string[];
  selection: ManagerProviderSelection;
  onToggle: (providerId: string) => void;
}) {
  const { t } = useI18n();
  if (!providers.length) {
    return (
      <PageEmpty
        title={t("managerNoProviders")}
        description={t("managerNoProvidersDescription")}
      />
    );
  }

  return (
    <ScrollPane
      className="min-h-0 flex-1"
      data-manager-provider-controls
      innerClassName="flex flex-col gap-2"
    >
      {providers.map((provider) => {
        const checked =
          selection === "all" || selected.includes(provider.id);
        return (
          <SelectableRowButton
            key={provider.id}
            title={provider.name}
            meta={provider.id}
            leading={<ProviderLogo providerId={provider.id} size="sm" alt={provider.name} />}
            selected={checked}
            trailing={checked ? <CheckIcon className="size-4 text-muted-foreground" aria-hidden /> : null}
            onClick={() => onToggle(provider.id)}
          />
        );
      })}
    </ScrollPane>
  );
}

function ManagerStatsStrip({
  stats,
  loading,
}: {
  stats: ReturnType<typeof useManagerStats>["data"];
  loading: boolean;
}) {
  const { t } = useI18n();
  const placeholder = loading ? <Skeleton className="h-5 w-20" /> : "-";
  return (
    <MetricGrid columns="four" data-manager-stats-strip>
      <MetricTile
        label={t("managerCurrentWorkspace")}
        value={stats ? formatBytes(stats.current_workspace_size_bytes) : placeholder}
        hint={t("managerSessionsCount", { count: stats?.current_workspace_session_count ?? 0 })}
        variant="compact"
      />
      <MetricTile
        label={t("managerAllWorkspaces")}
        value={stats ? stats.all_workspace_count : placeholder}
        hint={t("managerSessionsCount", { count: stats?.all_workspace_session_count ?? 0 })}
        variant="compact"
      />
      <MetricTile
        label={t("managerSelectedAgents")}
        value={stats?.selected_agent_count ?? placeholder}
        hint={t("managerInstalledProviders")}
        variant="compact"
      />
      <MetricTile
        label={t("managerAllSize")}
        value={stats ? formatBytes(stats.all_workspace_size_bytes) : placeholder}
        hint={t("managerIndexedStorage")}
        variant="compact"
      />
    </MetricGrid>
  );
}

function FilterToolbar({
  view,
  search,
  sort,
  visibleCount,
  hasActiveFilters,
  selection,
  onSearchChange,
  onSortChange,
  onSelectVisible,
  onClearFilters,
}: {
  view: ManagerView;
  search: string;
  sort: ManagerSort;
  visibleCount: number;
  hasActiveFilters: boolean;
  selection?: {
    count: number;
    visibleCount: number;
    bytes: number;
    onClear: () => void;
    onBackup: () => void;
    onDelete: () => void;
  } | null;
  onSearchChange: (value: string) => void;
  onSortChange: (sort: ManagerSort) => void;
  onSelectVisible: () => void;
  onClearFilters: () => void;
}) {
  const { t } = useI18n();
  return (
    <div
      className={
        selection
          ? "grid min-w-0 grid-cols-2 gap-2 border-t pt-3 sm:grid-cols-[minmax(14rem,1fr)_auto_minmax(0,1fr)_auto_auto_auto]"
          : "grid min-w-0 grid-cols-2 gap-2 border-t pt-3 sm:grid-cols-[minmax(14rem,1fr)_auto_auto_auto]"
      }
      data-manager-filter-toolbar
    >
      <div className="relative col-span-2 min-w-0 sm:col-span-1">
        <SearchIcon
          className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Input
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={
            view === "sessions"
              ? t("managerSearchSessions")
              : t("managerSearchWorkspaces")
          }
          className="min-h-10 pl-8"
          data-manager-search
        />
      </div>

      <Select
        value={sort}
        onValueChange={(value) => onSortChange(value as ManagerSort)}
      >
        <SelectTrigger className="min-h-10 w-full min-w-0" data-manager-sort>
          <SelectValue placeholder={t("managerSort")} />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="recent">{t("managerRecent")}</SelectItem>
            <SelectItem value="size">{t("managerSize")}</SelectItem>
            {view === "workspaces" ? (
              <SelectItem value="sessions">{t("managerSessionCount")}</SelectItem>
            ) : null}
            <SelectItem value="title">
              {view === "sessions" ? t("managerTitle") : t("managerName")}
            </SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>

      {selection ? (
        <>
          <div
            className="col-span-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 sm:col-span-1"
            data-manager-selection-bar
            aria-label={t("managerSelected", { count: selection.count })}
            role="group"
          >
            <strong className="text-sm">{t("managerSelected", { count: selection.count })}</strong>
            <span className="text-xs text-muted-foreground">
              {t("managerVisibleAndSize", { count: selection.visibleCount, size: formatBytes(selection.bytes) })}
            </span>
          </div>
          <Button
            type="button"
            variant="ghost"
            className="min-h-10 min-w-0"
            onClick={selection.onClear}
          >
            {t("managerClear")}
          </Button>
          <Button
            type="button"
            variant="outline"
            className="min-h-10 min-w-0"
            onClick={selection.onBackup}
          >
            <ArchiveIcon data-icon="inline-start" />
            {t("managerBackUp")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            className="min-h-10 min-w-0"
            onClick={selection.onDelete}
          >
            <Trash2Icon data-icon="inline-start" />
            {t("managerDelete")}
          </Button>
        </>
      ) : (
        <>
          <Button
            type="button"
            variant="outline"
            disabled={visibleCount === 0}
            onClick={onSelectVisible}
            className="min-h-10 min-w-0"
            data-manager-select-visible
          >
            <CheckIcon data-icon="inline-start" />
            {t("managerSelectVisible")}
          </Button>

          {hasActiveFilters ? (
            <Button
              type="button"
              variant="ghost"
              onClick={onClearFilters}
              className="min-h-10 min-w-0"
              data-manager-clear-filters
            >
              <XIcon data-icon="inline-start" />
              {t("managerClearFilters")}
            </Button>
          ) : null}
        </>
      )}
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
      className={`relative cursor-pointer rounded-md border px-3 py-3 transition-colors hover:border-foreground/20${selected ? " bg-muted" : ""}`}
      data-selected={selected ? "true" : "false"}
      data-manager-row
      aria-selected={selected}
      onClick={onToggle}
    >
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
        {children}
      </div>
      {selected ? (
        <CheckIcon
          className="pointer-events-none absolute right-3 bottom-3 size-4 text-muted-foreground"
          aria-hidden
        />
      ) : null}
    </article>
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
  const { t } = useI18n();
  return (
    <div
      className="flex flex-wrap items-center justify-start gap-2 md:justify-end"
      data-manager-row-actions
      onClick={(event) => event.stopPropagation()}
    >
      <Button asChild variant="outline">
        <Link to={href}>
          {t("managerView")}
          <ArrowRightIcon data-icon="inline-end" />
        </Link>
      </Button>
      <TrailingMoreButtonGroup
        trailingAction={
          <Button type="button" variant="outline" onClick={onDelete}>
            <Trash2Icon data-icon="inline-start" />
            {t("managerRemove")}
          </Button>
        }
        moreLabel={t("managerMoreActionsFor", { label })}
      >
        <DropdownMenuItem asChild>
          <Link to={href}>{t("managerOpen")}</Link>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onBackup}>
          <ArchiveIcon />
          {t("managerBackUp")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={onDelete}>
          <Trash2Icon />
          {t("managerDelete")}
        </DropdownMenuItem>
      </TrailingMoreButtonGroup>
    </div>
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
  onToggle: (item: ManagerItem) => void;
  onBackup: (item: ManagerItem) => void;
  onDelete: (item: ManagerItem) => void;
}) {
  const { t } = useI18n();
  if (!items.length) {
    return (
      <PageEmpty
        title={t("managerNoSessionsMatched")}
        description={t("managerNoSessionsMatchedDescription")}
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
          <RowShell
            key={item.id}
            selected={checked}
            onToggle={() => onToggle(item)}
          >
            <div className="flex min-w-0 flex-col gap-2">
              <Link
                to={href}
                className="truncate text-sm font-medium hover:underline"
                onClick={(event) => event.stopPropagation()}
              >
                {label}
              </Link>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <ProviderLogo providerId={item.provider_id} size="xs" alt={item.provider_name || item.provider_id} />
                  <span>{item.provider_name || item.provider_id}</span>
                </span>
                <span>{formatBytes(item.size_bytes)}</span>
                <span>{t("managerUpdated", { date: formatDateTime(item.last_active_at) })}</span>
              </div>
              <PathText
                value={item.project_dir || item.source_path}
                wrap="all"
              />
            </div>
            <RowActions
              href={href}
              label={label}
              onBackup={() => onBackup(item)}
              onDelete={() => onDelete(item)}
            />
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
  onBackup,
  onDelete,
}: {
  items: ManagerWorkspaceItem[];
  selected: Set<string>;
  onToggle: (item: ManagerWorkspaceItem) => void;
  onBackup: (item: ManagerWorkspaceItem) => void;
  onDelete: (item: ManagerWorkspaceItem) => void;
}) {
  const { t } = useI18n();
  if (!items.length) {
    return (
      <PageEmpty
        title={t("managerNoWorkspacesMatched")}
        description={t("managerNoWorkspacesMatchedDescription")}
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
          <RowShell
            key={identity}
            selected={checked}
            onToggle={() => onToggle(item)}
          >
            <div className="flex min-w-0 flex-col gap-2">
              <Link
                to={href}
                className="truncate text-sm font-medium hover:underline"
                onClick={(event) => event.stopPropagation()}
              >
                {label}
              </Link>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <ProviderLogo providerId={item.provider_id} size="xs" alt={item.provider_name || item.provider_id} />
                  <span>{item.provider_name || item.provider_id}</span>
                </span>
                <span>{t("managerSessionsCount", { count: item.session_count })}</span>
                <span>{formatBytes(item.total_size_bytes)}</span>
                <span>{t("managerUpdated", { date: formatDateTime(item.last_active_at) })}</span>
              </div>
              <span className="break-words font-mono text-xs text-muted-foreground">
                {item.workspace}
              </span>
            </div>
            <RowActions
              href={href}
              label={label}
              onBackup={() => onBackup(item)}
              onDelete={() => onDelete(item)}
            />
          </RowShell>
        );
      })}
    </div>
  );
}

export function ManagerPage() {
  const { t } = useI18n();
  const [searchParams, setSearchParams] = useSearchParams();
  const route = useMemo(
    () => readManagerRouteState(searchParams),
    [searchParams],
  );
  const selectedWorkspaceOverride = useUiStore(
    (state) => state.selectedWorkspace,
  );
  const [selectedSessions, setSelectedSessions] = useState<Map<string, ManagerItem>>(
    () => new Map(),
  );
  const [selectedWorkspaces, setSelectedWorkspaces] = useState<
    Map<string, ManagerWorkspaceItem>
  >(() => new Map());
  const [searchInput, setSearchInput] = useState(route.search);
  const [actionTarget, setActionTarget] = useState<ManagerActionTarget | null>(
    null,
  );
  const [actionReport, setActionReport] = useState<ManagerActionReport | null>(
    null,
  );
  const queryClient = useQueryClient();
  const meta = useManagerMeta();
  const currentWorkspace =
    selectedWorkspaceOverride || meta.data?.selected_workspace || null;
  const providerCatalog = useManagerProviderCatalog(currentWorkspace);
  const providerOptions = useMemo(
    () => managerProviderOptions(providerCatalog.data?.providers ?? []),
    [providerCatalog.data],
  );
  const availableProviderIds = useMemo(
    () => new Set(providerOptions.map((provider) => provider.id)),
    [providerOptions],
  );
  const selectedProviders = useMemo(
    () =>
      providerCatalog.data
        ? route.providers.filter((providerId) =>
            availableProviderIds.has(providerId),
          )
        : route.providers,
    [availableProviderIds, providerCatalog.data, route.providers],
  );
  const request = useMemo(
    () => resolveManagerRequest(route, currentWorkspace),
    [currentWorkspace, route],
  );
  const providersSelected = route.providerSelection !== "none";
  const listFilter = request.listFilter;
  const statsFilter = request.statsFilter;
  const stats = useManagerStats(statsFilter, {
    enabled: request.enabled && providersSelected,
  });
  const sessions = useManagerPreview(listFilter, {
    enabled: request.enabled && providersSelected && route.view === "sessions",
  });
  const workspaces = useManagerWorkspaces(listFilter, {
    enabled: request.enabled && providersSelected && route.view === "workspaces",
  });

  useEffect(() => {
    setSearchInput(route.search);
  }, [route.search]);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      if (searchInput === route.search) return;
      const next = new URLSearchParams(searchParams);
      resetManagerPageParam(next);
      if (searchInput.trim()) next.set("q", searchInput.trim());
      else next.delete("q");
      setSearchParams(next, { replace: true });
    }, MANAGER_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [searchInput, route.search, searchParams, setSearchParams]);

  const listTotalCount =
    route.view === "sessions"
      ? sessions.data?.total_count
      : workspaces.data?.total_count;

  useEffect(() => {
    if (listTotalCount === undefined) return;
    const loading =
      route.view === "sessions" ? sessions.isLoading : workspaces.isLoading;
    if (loading) return;

    const totalPages = managerTotalPages(listTotalCount, request.pageSize);
    if (request.page <= totalPages) return;

    const next = new URLSearchParams(searchParams);
    if (totalPages <= 1) next.delete("page");
    else next.set("page", String(totalPages));
    setSearchParams(next, { replace: true });
  }, [
    listTotalCount,
    request.page,
    request.pageSize,
    route.view,
    searchParams,
    sessions.isLoading,
    setSearchParams,
    workspaces.isLoading,
  ]);

  useEffect(() => {
    if (!providerCatalog.data) return;
    const expectedProvidersParam =
      route.providerSelection === "all"
        ? null
        : route.providerSelection === "none"
          ? "none"
          : selectedProviders.join(",");
    const currentProvidersParam = searchParams.has("providers")
      ? searchParams.get("providers")
      : null;

    if (currentProvidersParam === expectedProvidersParam) return;

    const next = new URLSearchParams(searchParams);
    if (expectedProvidersParam === null) next.delete("providers");
    else next.set("providers", expectedProvidersParam);
    setSearchParams(next, { replace: true });
  }, [
    providerCatalog.data,
    route.providerSelection,
    searchParams,
    selectedProviders,
    setSearchParams,
  ]);

  const managerAction = useMutation({
    mutationFn: async (
      target: ManagerActionTarget,
    ): Promise<ManagerActionReport> => {
      const outputDir = meta.data?.settings.default_backup_dir || "./backups";

      if (target.kind === "delete-sessions") {
        const result = await cleanManagerItems({ items: target.items });
        const outcome = actionOutcome(result.success, result.failed);
        return {
          title: actionResultTitle(
            outcome,
            t("managerDeletedTitle"),
            t("managerDeletePartialTitle"),
            t("managerDeleteFailureTitle"),
          ),
          summary: cleanSummary(result, t),
          lines: result.errors || [],
          outcome,
        };
      }

      if (target.kind === "backup-sessions") {
        const result = await backupManagerItems({
          items: target.items,
          output_dir: outputDir,
        });
        const outcome = actionOutcome(result.success, result.failed);
        return {
          title: actionResultTitle(
            outcome,
            t("managerBackedUpTitle"),
            t("managerBackupPartialTitle"),
            t("managerBackupFailureTitle"),
          ),
          summary: backupSummary(result, t),
          lines: [...(result.files || []), ...(result.errors || [])],
          outcome,
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
            t("managerUpdatedItemSummary", { provider: item.provider_id, workspace: workspaceName(item.workspace), success: result.success, failed: result.failed }),
          );
          errors.push(...(result.errors || []));
        }

        const outcome = actionOutcome(success, failed);
        return {
          title: actionResultTitle(
            outcome,
            t("managerWorkspaceDeletedTitle"),
            t("managerWorkspaceDeletePartialTitle"),
            t("managerWorkspaceDeleteFailureTitle"),
          ),
          summary: cleanSummary({
            success,
            failed,
            freed_bytes: freed,
            errors,
          }, t),
          lines: [...lines, ...errors],
          outcome,
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
          t("managerBackedUpItemSummary", { provider: item.provider_id, workspace: workspaceName(item.workspace), success: result.success, failed: result.failed }),
        );
        files.push(...(result.files || []));
        errors.push(...(result.errors || []));
      }

      const outcome = actionOutcome(success, failed);
      return {
        title: actionResultTitle(
          outcome,
          t("managerWorkspacesBackedUpTitle"),
          t("managerWorkspacesBackupPartialTitle"),
          t("managerWorkspacesBackupFailureTitle"),
        ),
        summary: backupSummary({ success, failed, files, errors }, t),
        lines: [...lines, ...files, ...errors],
        outcome,
      };
    },
    onSuccess: async (report) => {
      setActionTarget(null);
      setActionReport(report);
      if (report.outcome === "success") {
        setSelectedSessions(new Map());
        setSelectedWorkspaces(new Map());
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["manager"] }),
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
        queryClient.invalidateQueries({ queryKey: queryKeys.home }),
        queryClient.invalidateQueries({ queryKey: queryKeys.meta }),
      ]);
    },
    onError: (error, target) => {
      const message = error instanceof Error ? error.message : String(error);
      setActionTarget(null);
      setActionReport({
        title: `${actionTitle(target, t)} ${t("managerFailedSuffix")}`,
        summary: t("managerNoChangesRetry"),
        lines: [message],
        outcome: "failure",
      });
      toast.error(t("managerActionFailed"), { description: message });
    },
  });

  if (providerCatalog.isLoading || meta.isLoading) {
    return (
      <div data-manager-initial-loading aria-label={t("managerLoading")}>
        <PageSkeleton />
      </div>
    );
  }
  if (providerCatalog.error) {
    return (
      <PageError
        title={t("managerProvidersLoadFailed")}
        message={providerCatalog.error.message}
        onRetry={() => void providerCatalog.refetch()}
      />
    );
  }
  if (meta.error) {
    return (
      <PageError
        title={t("managerWorkspaceLoadFailed")}
        message={meta.error.message}
        onRetry={() => void meta.refetch()}
      />
    );
  }
  if (stats.error) {
    return (
      <PageError
        title={t("managerStatsLoadFailed")}
        message={stats.error.message}
        onRetry={() => void stats.refetch()}
      />
    );
  }
  if (route.view === "sessions" && sessions.error) {
    return (
      <PageError
        title={t("managerSessionsLoadFailed")}
        message={sessions.error.message}
        onRetry={() => void sessions.refetch()}
      />
    );
  }
  if (route.view === "workspaces" && workspaces.error) {
    return (
      <PageError
        title={t("managerWorkspacesLoadFailed")}
        message={workspaces.error.message}
        onRetry={() => void workspaces.refetch()}
      />
    );
  }

  const sessionRows = sessions.data?.items ?? [];
  const workspaceRows = workspaces.data?.items ?? [];
  const selectedSessionItems = Array.from(selectedSessions.values());
  const selectedWorkspaceItems = Array.from(selectedWorkspaces.values());
  const selectedSessionIds = new Set(selectedSessions.keys());
  const selectedWorkspaceIds = new Set(selectedWorkspaces.keys());
  const selectedSessionBytes = selectedSessionItems.reduce(
    (sum, item) => sum + item.size_bytes,
    0,
  );
  const selectedWorkspaceBytes = selectedWorkspaceItems.reduce(
    (sum, item) => sum + item.total_size_bytes,
    0,
  );
  const visibleSelectedSessions = sessionRows.filter((item) =>
    selectedSessionIds.has(item.id),
  ).length;
  const visibleSelectedWorkspaces = workspaceRows.filter((item) =>
    selectedWorkspaceIds.has(workspaceIdentity(item)),
  ).length;
  const resultRows = route.view === "sessions" ? sessionRows : workspaceRows;
  const totalCount = listTotalCount;
  const selectedCount =
    route.view === "sessions"
      ? selectedSessionItems.length
      : selectedWorkspaceItems.length;
  const hasNarrowingFilters =
    route.providerSelection !== "all" || Boolean(route.search.trim());
  const hasActiveFilters = hasNarrowingFilters || route.sort !== "recent";
  const activeResults = route.view === "sessions" ? sessions : workspaces;
  const initialResultsLoading = activeResults.isLoading && !activeResults.data;
  const listPageFetching =
    activeResults.isFetching && Boolean(activeResults.data);
  const resultsRefreshing =
    !initialResultsLoading && (stats.isFetching || listPageFetching);
  const scopeIsEmpty =
    !initialResultsLoading && totalCount === 0 && !hasNarrowingFilters;
  const filtersAreEmpty =
    !initialResultsLoading && resultRows.length === 0 && !scopeIsEmpty;
  const listBusyLabel = listPageFetching
    ? t("managerLoadingPage")
    : resultsRefreshing
      ? t("managerRefreshingResults")
      : null;

  function replaceRoute(
    update: (next: URLSearchParams) => void,
    replace = false,
  ) {
    const next = new URLSearchParams(searchParams);
    update(next);
    setSearchParams(next, { replace });
  }

  function clearSelection() {
    setSelectedSessions(new Map());
    setSelectedWorkspaces(new Map());
  }

  function changeScope(scope: ManagerScope) {
    replaceRoute((next) => {
      next.delete("workspace");
      resetManagerPageParam(next);
      if (scope === "all") {
        next.set("scope", "all");
        next.set("view", "workspaces");
      } else {
        next.delete("scope");
        next.delete("view");
        if (route.sort === "sessions") next.delete("sort");
      }
    });
    clearSelection();
  }

  function changeSort(sort: ManagerSort) {
    replaceRoute((next) => {
      resetManagerPageParam(next);
      if (sort === "recent") next.delete("sort");
      else next.set("sort", sort);
    });
    clearSelection();
  }

  function setProviderSelection(
    selection: ManagerProviderSelection,
    customProviders: string[] = [],
  ) {
    replaceRoute((next) => {
      resetManagerPageParam(next);
      if (selection === "all") next.delete("providers");
      else if (selection === "none") next.set("providers", "none");
      else if (customProviders.length) {
        next.set("providers", customProviders.join(","));
      } else {
        next.set("providers", "none");
      }
    });
    clearSelection();
  }

  function toggleProvider(providerId: string) {
    if (route.providerSelection === "all") {
      setProviderSelection("custom", [providerId]);
      return;
    }

    if (route.providerSelection === "none") {
      setProviderSelection("custom", [providerId]);
      return;
    }

    if (selectedProviders.includes(providerId)) {
      const nextProviders = selectedProviders.filter((id) => id !== providerId);
      if (nextProviders.length === 0) {
        setProviderSelection("none");
      } else if (nextProviders.length === providerOptions.length) {
        setProviderSelection("all");
      } else {
        setProviderSelection("custom", nextProviders);
      }
      return;
    }

    const nextProviders = [...selectedProviders, providerId];
    if (nextProviders.length === providerOptions.length) {
      setProviderSelection("all");
    } else {
      setProviderSelection("custom", nextProviders);
    }
  }

  function clearFilters() {
    replaceRoute((next) => {
      next.delete("providers");
      next.delete("q");
      next.delete("sort");
      resetManagerPageParam(next);
    });
    setSearchInput("");
    clearSelection();
  }

  function toggleSession(item: ManagerItem) {
    setSelectedSessions((current) => {
      const next = new Map(current);
      if (next.has(item.id)) next.delete(item.id);
      else next.set(item.id, item);
      return next;
    });
  }

  function toggleWorkspace(item: ManagerWorkspaceItem) {
    const identity = workspaceIdentity(item);
    setSelectedWorkspaces((current) => {
      const next = new Map(current);
      if (next.has(identity)) next.delete(identity);
      else next.set(identity, item);
      return next;
    });
  }

  function selectVisible() {
    if (route.view === "sessions") {
      setSelectedSessions((current) => {
        const next = new Map(current);
        for (const item of sessionRows) {
          next.set(item.id, item);
        }
        return next;
      });
    } else {
      setSelectedWorkspaces((current) => {
        const next = new Map(current);
        for (const item of workspaceRows) {
          next.set(workspaceIdentity(item), item);
        }
        return next;
      });
    }
  }

  function changePage(page: number) {
    replaceRoute((next) => {
      if (page <= 1) next.delete("page");
      else next.set("page", String(page));
    });
  }

  function changePageSize(pageSize: ManagerPageSize) {
    replaceRoute((next) => {
      resetManagerPageParam(next);
      if (pageSize === DEFAULT_MANAGER_PAGE_SIZE) next.delete("pageSize");
      else next.set("pageSize", String(pageSize));
    });
  }

  function refreshResults() {
    void Promise.all([stats.refetch(), activeResults.refetch()]);
  }

  function openAction(target: ManagerActionTarget) {
    if (!target.items.length) {
      toast.error(t("managerNoSelection"));
      return;
    }
    setActionTarget(target);
  }

  const deleteTarget = isDeleteAction(actionTarget) ? actionTarget : null;
  const backupTarget = isBackupAction(actionTarget) ? actionTarget : null;
  const backupDir = meta.data?.settings.default_backup_dir || "./backups";

  return (
    <>
      <TwoPanePage className="h-full min-h-0 flex-1" data-manager-page-layout>
        <PanelCard className="flex h-full min-h-0 flex-col overflow-hidden" data-manager-control-panel>
          <section
            className="flex flex-col gap-3 border-b pb-4"
            data-manager-workspace-summary
          >
            <WorkspaceIdentity
              workspace={request.workspace}
              fallbackTitle={route.scope === "all" ? t("allWorkspaces") : t("workspaceNoWorkspace")}
              titleClassName="mt-1 block text-lg leading-tight"
              pathClassName="mt-1"
            />
          </section>

          <section className="flex min-h-0 flex-1 flex-col gap-3 pt-3">
            <ProviderFilter
              providers={providerOptions}
              selected={selectedProviders}
              selection={route.providerSelection}
              onToggle={toggleProvider}
            />
          </section>
        </PanelCard>

        <PanelCard
          variant="plain"
          className="flex h-full min-h-0 flex-col gap-4 overflow-hidden"
          data-manager-result-panel
        >
          <div className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-2">
              <strong className="text-sm font-medium">
                {route.view === "sessions"
                  ? t("managerSessionPreview")
                  : t("managerWorkspacePreview")}
              </strong>
              <div className="flex items-center gap-2">
                {listBusyLabel ? (
                  <span
                    className="inline-flex items-center gap-1 text-xs text-muted-foreground"
                    data-manager-refreshing
                  >
                    <LoaderCircleIcon
                      className="size-3.5 animate-spin"
                      aria-hidden="true"
                    />
                    {listBusyLabel}
                  </span>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  className="min-h-10"
                  onClick={() =>
                    changeScope(route.scope === "all" ? "current" : "all")
                  }
                  data-manager-scope-toggle
                >
                  {route.scope === "all"
                    ? t("managerSwitchToCurrent")
                    : t("managerSwitchToAll")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  className="min-h-10"
                  onClick={refreshResults}
                  disabled={!request.enabled || resultsRefreshing}
                  data-manager-refresh
                >
                  <RefreshCwIcon data-icon="inline-start" />
                  {t("managerRefresh")}
                </Button>
              </div>
            </div>

            <ManagerStatsStrip stats={stats.data} loading={stats.isLoading} />

            <FilterToolbar
              view={route.view}
              search={searchInput}
              sort={route.sort}
              visibleCount={resultRows.length}
              hasActiveFilters={hasActiveFilters}
              selection={
                selectedCount > 0
                  ? route.view === "sessions"
                    ? {
                        count: selectedSessionItems.length,
                        visibleCount: visibleSelectedSessions,
                        bytes: selectedSessionBytes,
                        onClear: () => setSelectedSessions(new Map()),
                        onBackup: () =>
                          openAction({
                            kind: "backup-sessions",
                            items: selectedSessionItems,
                          }),
                        onDelete: () =>
                          openAction({
                            kind: "delete-sessions",
                            items: selectedSessionItems,
                          }),
                      }
                    : {
                        count: selectedWorkspaceItems.length,
                        visibleCount: visibleSelectedWorkspaces,
                        bytes: selectedWorkspaceBytes,
                        onClear: () => setSelectedWorkspaces(new Map()),
                        onBackup: () =>
                          openAction({
                            kind: "backup-workspaces",
                            items: selectedWorkspaceItems,
                          }),
                        onDelete: () =>
                          openAction({
                            kind: "delete-workspaces",
                            items: selectedWorkspaceItems,
                          }),
                      }
                  : null
              }
              onSearchChange={setSearchInput}
              onSortChange={changeSort}
              onSelectVisible={selectVisible}
              onClearFilters={clearFilters}
            />
          </div>

          <ScrollPane className="min-h-0 flex-1">
            {!request.enabled ? (
              <PageEmpty
                title={t("managerNoCurrentWorkspace")}
                description={t("managerNoCurrentWorkspaceDescription")}
              />
            ) : !providersSelected ? (
              <PageEmpty
                title={t("managerNoProvidersSelected")}
                description={t("managerNoProvidersSelectedDescription")}
              />
            ) : initialResultsLoading ? (
              <div
                data-manager-results-loading
                aria-label={t("managerLoadingResults")}
              >
                <PageSkeleton />
              </div>
            ) : scopeIsEmpty ? (
              <PageEmpty
                title={
                  route.view === "sessions"
                    ? t("managerNoSessionsInScope")
                    : t("managerNoWorkspacesInScope")
                }
                description={
                  route.scope === "all"
                    ? t("managerNoIndexedAll")
                    : t("managerNoIndexedCurrent")
                }
                onRefresh={refreshResults}
              />
            ) : filtersAreEmpty ? (
              <PageEmpty
                title={
                  route.view === "sessions"
                    ? t("managerNoSessionsFiltered")
                    : t("managerNoWorkspacesFiltered")
                }
                description={t("managerClearOrChangeFilters")}
                onRefresh={refreshResults}
              />
            ) : (
              <div
                className={
                  listPageFetching
                    ? "opacity-60 transition-opacity"
                    : "transition-opacity"
                }
                data-manager-result-list
                aria-busy={listPageFetching || undefined}
              >
                {route.view === "sessions" ? (
                  <SessionRows
                    items={sessionRows}
                    selected={selectedSessionIds}
                    onToggle={toggleSession}
                    onBackup={(item) =>
                      openAction({ kind: "backup-sessions", items: [item] })
                    }
                    onDelete={(item) =>
                      openAction({ kind: "delete-sessions", items: [item] })
                    }
                  />
                ) : (
                  <WorkspaceRows
                    items={workspaceRows}
                    selected={selectedWorkspaceIds}
                    onToggle={toggleWorkspace}
                    onBackup={(item) =>
                      openAction({ kind: "backup-workspaces", items: [item] })
                    }
                    onDelete={(item) =>
                      openAction({ kind: "delete-workspaces", items: [item] })
                    }
                  />
                )}
              </div>
            )}
          </ScrollPane>

          {request.enabled && !initialResultsLoading && (totalCount ?? 0) > 0 ? (
            <ManagerResultPagination
              page={request.page}
              pageSize={request.pageSize}
              totalCount={totalCount ?? 0}
              onPageChange={changePage}
              onPageSizeChange={changePageSize}
            />
          ) : null}
        </PanelCard>
      </TwoPanePage>

      <AlertDialog
        open={Boolean(deleteTarget)}
        onOpenChange={(open) => {
          if (!open && !managerAction.isPending) setActionTarget(null);
        }}
      >
        <AlertDialogContent
          className="max-w-[calc(100vw-2rem)] sm:max-w-lg"
          onEscapeKeyDown={(event) => {
            if (managerAction.isPending) event.preventDefault();
          }}
          data-manager-action-dialog
          data-manager-delete-dialog
        >
          <AlertDialogHeader>
            <AlertDialogMedia className="bg-destructive/10 text-destructive">
              <Trash2Icon />
            </AlertDialogMedia>
            <AlertDialogTitle>{actionTitle(deleteTarget, t)}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("managerReviewDelete")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {deleteTarget ? <ActionTargetSummary target={deleteTarget} /> : null}
          <AlertDialogFooter>
            <AlertDialogCancel
              className="min-h-10"
              disabled={managerAction.isPending}
            >
              {t("managerCancel")}
            </AlertDialogCancel>
            <Button
              type="button"
              variant="destructive"
              className="min-h-10"
              onClick={() => deleteTarget && managerAction.mutate(deleteTarget)}
              disabled={!deleteTarget || managerAction.isPending}
              data-manager-confirm-delete
            >
              {managerAction.isPending ? (
                <>
                  <LoaderCircleIcon
                    className="animate-spin"
                    data-icon="inline-start"
                  />
                  {t("managerDeleting")}
                </>
              ) : (
                <>
                  <Trash2Icon data-icon="inline-start" />
                  {t("managerDeletePermanently")}
                </>
              )}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <Dialog
        open={Boolean(backupTarget)}
        onOpenChange={(open) => {
          if (!open && !managerAction.isPending) setActionTarget(null);
        }}
      >
        <DialogContent
          className="sm:max-w-lg"
          showCloseButton={!managerAction.isPending}
          onEscapeKeyDown={(event) => {
            if (managerAction.isPending) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (managerAction.isPending) event.preventDefault();
          }}
          data-manager-action-dialog
          data-manager-backup-dialog
        >
          <DialogHeader>
            <DialogTitle>{actionTitle(backupTarget, t)}</DialogTitle>
            <DialogDescription>
              {t("managerConfirmBackup")}
            </DialogDescription>
          </DialogHeader>
          {backupTarget ? (
            <ActionTargetSummary target={backupTarget} backupDir={backupDir} />
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              className="min-h-10"
              onClick={() => setActionTarget(null)}
              disabled={managerAction.isPending}
            >
              {t("managerCancel")}
            </Button>
            <Button
              type="button"
              className="min-h-10"
              onClick={() => backupTarget && managerAction.mutate(backupTarget)}
              disabled={!backupTarget || managerAction.isPending}
              data-manager-confirm-backup
            >
              {managerAction.isPending ? (
                <>
                  <LoaderCircleIcon
                    className="animate-spin"
                    data-icon="inline-start"
                  />
                  {t("managerBackingUp")}
                </>
              ) : (
                <>
                  <ArchiveIcon data-icon="inline-start" />
                  {t("managerStartBackup")}
                </>
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(actionReport)}
        onOpenChange={(open) => !open && setActionReport(null)}
      >
        <DialogContent
          className="sm:max-w-lg"
          showCloseButton={false}
          data-manager-action-result
          data-manager-action-outcome={actionReport?.outcome}
        >
          <DialogHeader>
            {actionReport ? (
              <Badge
                variant={
                  actionReport.outcome === "failure"
                    ? "destructive"
                    : actionReport.outcome === "partial"
                      ? "secondary"
                      : "default"
                }
              >
                {actionReport.outcome === "success"
                  ? t("managerCompleted")
                  : actionReport.outcome === "partial"
                    ? t("managerPartiallyCompleted")
                    : t("managerFailed")}
              </Badge>
            ) : null}
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
            <Button
              type="button"
              className="min-h-10"
              onClick={() => setActionReport(null)}
            >
              {t("managerClose")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
