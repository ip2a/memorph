import { GitBranchIcon, RotateCwIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SectionHeading } from "@/components/shared/section-heading";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { useManagerMeta } from "@/features/manager/queries";
import { SyncGroupDetailPanel } from "@/features/sync/sync-detail-page";
import { useSyncGroups } from "@/features/sync/queries";
import { formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SyncGroup } from "@/lib/types";

function latestHolding(group: SyncGroup) {
  return [...group.holdings].sort((left, right) => (right.last_active_at ?? 0) - (left.last_active_at ?? 0))[0];
}

function matchesSyncGroupSearch(group: SyncGroup, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;

  const latest = latestHolding(group);
  return [
    group.title,
    group.id,
    group.source_provider,
    latest?.provider,
    latest?.session_id,
    latest?.target_dir,
  ].some((value) => value?.toLowerCase().includes(normalized));
}

function matchesSyncGroupWorkspace(group: SyncGroup, workspace: string | null | undefined) {
  if (!workspace) return true;
  const dirs = group.holdings.map((holding) => holding.target_dir).filter(Boolean);
  if (!dirs.length) return true;
  return dirs.some((dir) => dir === workspace || dir?.startsWith(`${workspace}/`));
}

function SyncGroupRow({
  group,
  selected,
  onSelect,
}: {
  group: SyncGroup;
  selected: boolean;
  onSelect: (groupId: string) => void;
}) {
  const { t } = useI18n();
  const latest = latestHolding(group);
  const errorHoldings = group.holdings.filter((holding) => holding.last_error);

  return (
    <SelectableRowButton
      data-sync-group-row
      selected={selected}
      leading={<GitBranchIcon className="size-4 shrink-0" aria-hidden />}
      title={group.title || group.id}
      meta={(
        <span className="flex flex-col gap-0.5">
          <span className="flex flex-wrap items-center gap-2">
            {group.source_provider ? (
              <>
                <ProviderLogo providerId={group.source_provider} size="xs" alt={group.source_provider} />
                <span>{group.source_provider}</span>
              </>
            ) : null}
            <span>{t("syncHoldingsCount", { count: group.holdings.length })}</span>
            <span>{t("syncUpdatedAt", { date: formatDateTime(group.updated_at) })}</span>
          </span>
          <span className="truncate font-mono text-[11px]">
            {t("syncLatestHolding", { holding: latest ? `${latest.provider}:${latest.session_id}` : "-" })}
          </span>
          {latest?.target_dir ? (
            <PathText value={latest.target_dir} fallback="-" wrap="all" className="text-[11px]" />
          ) : null}
        </span>
      )}
      trailing={errorHoldings.length ? <Badge variant="destructive">{errorHoldings.length}</Badge> : null}
      onClick={() => onSelect(group.id)}
    />
  );
}

export function SyncPage() {
  const { t } = useI18n();
  const [searchParams, setSearchParams] = useSearchParams();
  const syncGroups = useSyncGroups();
  const meta = useManagerMeta();
  const [search, setSearch] = useState("");

  const groups = useMemo(() => syncGroups.data ?? [], [syncGroups.data]);
  const filteredGroups = useMemo(
    () =>
      groups.filter(
        (group) =>
          matchesSyncGroupWorkspace(group, meta.data?.selected_workspace) &&
          matchesSyncGroupSearch(group, search),
      ),
    [groups, meta.data?.selected_workspace, search],
  );

  const selectedGroupId = searchParams.get("group") ?? "";
  const selectedGroup = filteredGroups.find((group) => group.id === selectedGroupId) ?? null;

  useEffect(() => {
    if (!filteredGroups.length) return;
    if (selectedGroupId && filteredGroups.some((group) => group.id === selectedGroupId)) return;
    setSearchParams({ group: filteredGroups[0].id }, { replace: true });
  }, [filteredGroups, selectedGroupId, setSearchParams]);

  if (syncGroups.isLoading || meta.isLoading) return <PageSkeleton />;
  if (syncGroups.error) return <PageError title={t("syncGroupsLoadFailed")} message={syncGroups.error.message} />;
  if (meta.error) return <PageError title={t("syncWorkspaceLoadFailed")} message={meta.error.message} />;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TwoPanePage className="min-h-0 flex-1" data-sync-page-layout>
        <PanelCard className="flex min-h-0 flex-col gap-3 p-3" data-sync-group-panel>
          <div className="flex flex-col gap-3 border-b pb-3">
            <SectionHeading title={t("syncGroups")} badge={filteredGroups.length} />
            <div className="relative w-full">
              <SearchIcon className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
              <Input
                className="pl-8"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t("syncSearchPlaceholder")}
                data-sync-preview-search
              />
            </div>
            <Button variant="outline" onClick={() => syncGroups.refetch()} disabled={syncGroups.isFetching}>
              {syncGroups.isFetching ? <Spinner data-icon="inline-start" /> : <RotateCwIcon data-icon="inline-start" />}
              {t("refresh")}
            </Button>
          </div>
          <ScrollArea className="min-h-0 flex-1 pr-3">
            <div className="flex flex-col gap-2">
              {filteredGroups.length ? (
                filteredGroups.map((group) => (
                  <SyncGroupRow
                    key={group.id}
                    group={group}
                    selected={group.id === selectedGroupId}
                    onSelect={(groupId) => setSearchParams({ group: groupId })}
                  />
                ))
              ) : (
                <PageEmpty
                  title={groups.length ? t("syncNoMatches") : t("syncNoGroups")}
                  description={groups.length ? t("syncNoMatchesDescription") : t("syncNoGroupsDescription")}
                />
              )}
            </div>
          </ScrollArea>
        </PanelCard>

        {selectedGroup ? (
          <SyncGroupDetailPanel
            groupId={selectedGroup.id}
            onRemoved={() => setSearchParams({})}
          />
        ) : (
          <PanelCard className="flex min-h-0 flex-col gap-3 p-3" data-sync-detail-panel>
            <PageEmpty title={t("syncSelectGroup")} description={t("syncSelectGroupDescription")} />
          </PanelCard>
        )}
      </TwoPanePage>
    </div>
  );
}
