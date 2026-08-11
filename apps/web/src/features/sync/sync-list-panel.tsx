import { useMemo } from "react";
import { Link } from "react-router-dom";
import { GitBranchIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useManagerMeta } from "@/features/manager/queries";
import { useSyncGroups } from "@/features/sync/queries";
import { formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SyncGroup } from "@/lib/types";

function latestHolding(group: SyncGroup) {
  return [...group.holdings].sort(
    (left, right) => (right.last_active_at ?? 0) - (left.last_active_at ?? 0),
  )[0];
}

function matchesSyncGroupWorkspace(
  group: SyncGroup,
  workspace: string | null | undefined,
) {
  if (!workspace) return true;
  const dirs = group.holdings
    .map((holding) => holding.target_dir)
    .filter(Boolean);
  if (!dirs.length) return true;
  return dirs.some(
    (dir) => dir === workspace || dir?.startsWith(`${workspace}/`),
  );
}

export function SyncListPanel() {
  const { t } = useI18n();
  const meta = useManagerMeta();
  const syncGroups = useSyncGroups();

  const groups = useMemo(() => syncGroups.data ?? [], [syncGroups.data]);
  const filteredGroups = useMemo(
    () =>
      groups.filter((group) =>
        matchesSyncGroupWorkspace(group, meta.data?.selected_workspace),
      ),
    [groups, meta.data?.selected_workspace],
  );

  if (syncGroups.isLoading || meta.isLoading) return <PageSkeleton />;
  if (syncGroups.error) {
    return (
      <PageError
        title={t("syncGroupsLoadFailed")}
        message={syncGroups.error.message}
      />
    );
  }
  if (meta.error) {
    return (
      <PageError
        title={t("syncWorkspaceLoadFailed")}
        message={meta.error.message}
      />
    );
  }

  if (!filteredGroups.length) {
    return (
      <PageEmpty
        title={groups.length ? t("syncNoMatches") : t("syncNoGroups")}
        description={
          groups.length
            ? t("syncNoMatchesDescription")
            : t("syncNoGroupsDescription")
        }
      />
    );
  }

  return (
    <div className="flex flex-col gap-2" data-sync-list-panel>
      {filteredGroups.map((group) => {
        const href = `/sync?group=${encodeURIComponent(group.id)}`;
        const latest = latestHolding(group);
        const errorHoldings = group.holdings.filter(
          (holding) => holding.last_error,
        );
        return (
          <EntityRow
            key={group.id}
            data-sync-group-row
            actions={
              <Button asChild variant="outline" size="sm">
                <Link to={href}>{t("view")}</Link>
              </Button>
            }
          >
            <div className="flex min-w-0 flex-col gap-1.5">
              <Link
                to={href}
                className="flex items-center gap-2 truncate text-sm font-medium hover:underline"
              >
                <GitBranchIcon className="size-3.5 shrink-0" aria-hidden />
                {group.title || group.id}
              </Link>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                {group.source_provider ? (
                  <span className="inline-flex items-center gap-1.5">
                    <ProviderLogo
                      providerId={group.source_provider}
                      size="xs"
                      alt={group.source_provider}
                    />
                    <span>{group.source_provider}</span>
                  </span>
                ) : null}
                <span>
                  {t("syncHoldingsCount", { count: group.holdings.length })}
                </span>
                <span>{formatDateTime(group.updated_at)}</span>
                {latest ? (
                  <span className="truncate font-mono">
                    {latest.provider}:{latest.session_id}
                  </span>
                ) : null}
                {errorHoldings.length ? (
                  <Badge variant="destructive">{errorHoldings.length}</Badge>
                ) : null}
              </div>
            </div>
          </EntityRow>
        );
      })}
    </div>
  );
}