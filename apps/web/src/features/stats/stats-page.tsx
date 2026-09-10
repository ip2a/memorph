import { useMemo, useState } from "react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  ActivityTrend,
  RankingBoard,
  StatsCompositionPanel,
  StatsOverviewPanel,
  type StatsOverviewMetric,
} from "@/features/stats/stats-overview-panels";
import {
  type StatsWorkspaceScope,
  useStatsDashboard,
} from "@/features/stats/queries";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import { resolvePreferredStatsDashboardRange } from "@/lib/custom-range-preferences";
import type { StatsDashboardRange } from "@/lib/types";

const RANGE_HINT_KEYS: Record<StatsDashboardRange, I18nKey> = {
  "7d": "statsRange7d",
  "30d": "statsRange30d",
  "90d": "statsRange90d",
  all: "statsRangeAll",
};

export function StatsPage() {
  const { t } = useI18n();
  const [range, setRange] = useState<StatsDashboardRange>(() =>
    resolvePreferredStatsDashboardRange(),
  );
  const [scope, setScope] = useState<StatsWorkspaceScope>("workspace");
  const { dashboard, meta, all } = useStatsDashboard(range, scope);

  const period = t(RANGE_HINT_KEYS[range]);

  const overviewMetrics = useMemo<StatsOverviewMetric[]>(() => {
    if (!dashboard.data) return [];
    const data = dashboard.data;
    return [
      {
        label: t("statsTotalSessions"),
        value: data.overview.total_sessions.toLocaleString(),
        hint: t("statsCumulativeNew", {
          count: data.overview.new_sessions.toLocaleString(),
        }),
      },
      {
        label: t("statsActiveSessions"),
        value: data.overview.active_sessions.toLocaleString(),
        hint: period,
      },
      {
        label: t("statsTotalMessages"),
        value: data.overview.total_messages.toLocaleString(),
        hint: data.overview.unknown_message_counts
          ? t("statsSessionsUncounted", {
              count: data.overview.unknown_message_counts.toLocaleString(),
            })
          : t("statsActiveSessionMessagesHint", {
              count: data.overview.active_session_messages.toLocaleString(),
            }),
      },
      {
        label: t("statsDataUsage"),
        value: formatBytes(data.overview.total_size_bytes),
        hint: data.overview.unknown_size_bytes
          ? t("statsSessionsSizeUnknown", {
              count: data.overview.unknown_size_bytes.toLocaleString(),
            })
          : t("statsStaleSize", {
              size: formatBytes(data.overview.stale_size_bytes),
            }),
      },
      {
        label: t("statsInactiveOver90d"),
        value: data.attention.inactive_over_90d.count.toLocaleString(),
        hint: t("statsOccupies", {
          size: formatBytes(data.attention.inactive_over_90d.size_bytes),
        }),
      },
      {
        label: t("statsLargeSessions"),
        value: data.attention.large_sessions.count.toLocaleString(),
        hint: t("statsOccupies", {
          size: formatBytes(data.attention.large_sessions.size_bytes),
        }),
      },
      {
        label: t("statsShortSessions"),
        value: data.attention.short_sessions.count.toLocaleString(),
        hint: t("statsOccupies", {
          size: formatBytes(data.attention.short_sessions.size_bytes),
        }),
      },
    ];
  }, [dashboard.data, period, t]);

  if (meta.isLoading || dashboard.isLoading) return <PageSkeleton />;
  if (meta.error || dashboard.error) {
    return (
      <PageError
        title={t("statsLoadFailed")}
        message={
          (meta.error ?? dashboard.error)?.message ?? t("statsUnknownError")
        }
      />
    );
  }
  if (!dashboard.data) {
    return (
      <PageError
        title={t("statsNoStatistics")}
        message={t("statsSelectWorkspace")}
      />
    );
  }

  const data = dashboard.data;

  return (
    <ScrollPane
      className="min-h-0 flex-1 size-full"
      data-stats-page
      innerClassName="flex min-w-0 flex-col gap-6 pb-6"
    >
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Tabs
            value={scope}
            onValueChange={(value) => setScope(value as StatsWorkspaceScope)}
          >
            <TabsList>
              <TabsTrigger value="workspace">{t("statsScopeWorkspace")}</TabsTrigger>
              <TabsTrigger value="all">{t("statsScopeAll")}</TabsTrigger>
            </TabsList>
          </Tabs>
          <Tabs
            value={range}
            onValueChange={(value) => setRange(value as StatsDashboardRange)}
          >
            <TabsList>
              <TabsTrigger value="7d">{t("skillsDays", { count: 7 })}</TabsTrigger>
              <TabsTrigger value="30d">{t("skillsDays", { count: 30 })}</TabsTrigger>
              <TabsTrigger value="90d">{t("skillsDays", { count: 90 })}</TabsTrigger>
              <TabsTrigger value="all">{t("statsAllRange")}</TabsTrigger>
            </TabsList>
          </Tabs>
        </div>

        <section className="@container/stats-panel grid min-w-0 grid-cols-10 items-stretch gap-4">
          <div className="col-span-4 min-w-0" data-stats-overview>
            <StatsOverviewPanel metrics={overviewMetrics} />
          </div>

          <div className="col-span-6 min-w-0">
            <StatsCompositionPanel data={data} />
          </div>
        </section>

        <ActivityTrend
          data={data.timeline}
          unknownMessageTimestamps={data.overview.unknown_message_timestamps}
        />

        <RankingBoard
          sessions={data.top_sessions}
          providers={data.providers}
          workspaces={data.workspaces}
          all={all}
        />
    </ScrollPane>
  );
}
