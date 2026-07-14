import { useState } from "react";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { SectionHeading } from "@/components/shared/section-heading";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { StatsOverviewPanels } from "@/features/stats/stats-overview-panels";
import { type StatsRange, type StatsWorkspaceScope, useStatsDashboard } from "@/features/stats/queries";
import { formatBytes } from "@/lib/format";

export function StatsPage() {
  const [range, setRange] = useState<StatsRange>("7d");
  const [scope, setScope] = useState<StatsWorkspaceScope>("workspace");
  const dashboard = useStatsDashboard(range, scope);
  const {
    activityLoading,
    activityTimeline,
    activityTotal,
    allWorkspaces,
    hooks,
    hours,
    loading,
    providerBreakdown,
    rankBarItems,
    sessions,
    stats,
    error,
  } = dashboard;

  const hookSummary = hooks.data?.summary;
  const placeholder = loading ? <Skeleton className="h-5 w-16" /> : "-";
  const activityValue =
    activityLoading || loading
      ? placeholder
      : Number.isInteger(activityTotal)
        ? String(activityTotal)
        : activityTotal.toFixed(1);

  const usageTable = {
    items: rankBarItems,
    labelColumn: allWorkspaces ? "Workspace" : "Session",
    valueColumn: "Messages",
  };

  if (loading && !stats.data && !hooks.data) {
    return <PageSkeleton />;
  }

  if (error) {
    return <PageError title="统计加载失败" message={error.message} />;
  }

  return (
    <ScrollArea className="h-full" data-stats-page>
      <div className="flex flex-col gap-4 pb-4">
        <SectionHeading
          variant="page"
          title="统计概览"
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <Tabs value={scope} onValueChange={(value) => setScope(value as StatsWorkspaceScope)}>
                <TabsList>
                  <TabsTrigger value="workspace">当前工作空间</TabsTrigger>
                  <TabsTrigger value="all">全部工作空间</TabsTrigger>
                </TabsList>
              </Tabs>
              <Tabs value={range} onValueChange={(value) => setRange(value as StatsRange)}>
                <TabsList>
                  <TabsTrigger value="24h">24h</TabsTrigger>
                  <TabsTrigger value="7d">7d</TabsTrigger>
                  <TabsTrigger value="30d">30d</TabsTrigger>
                  <TabsTrigger value="all">全部</TabsTrigger>
                </TabsList>
              </Tabs>
            </div>
          }
        />

        <section className="border-b pb-4" data-stats-kpi-strip>
          <MetricGrid columns="auto" className="grid-cols-3 justify-items-center xl:grid-cols-6">
            <MetricTile
              label="Sessions"
              value={
                stats.data
                  ? allWorkspaces
                    ? stats.data.all_workspace_session_count
                    : stats.data.current_workspace_session_count
                  : placeholder
              }
              hint={
                stats.data
                  ? allWorkspaces
                    ? `${stats.data.all_workspace_count} workspaces`
                    : `${stats.data.all_workspace_session_count} all workspaces`
                  : allWorkspaces
                    ? "all workspaces"
                    : "current workspace"
              }
              variant="square"
            />
            <MetricTile
              label="Storage"
              value={
                stats.data
                  ? formatBytes(
                      allWorkspaces ? stats.data.all_workspace_size_bytes : stats.data.current_workspace_size_bytes,
                    )
                  : placeholder
              }
              hint={
                stats.data
                  ? allWorkspaces
                    ? `${stats.data.all_workspace_count} workspaces indexed`
                    : `${formatBytes(stats.data.all_workspace_size_bytes)} all`
                  : "indexed size"
              }
              variant="square"
            />
            <MetricTile
              label="Providers"
              value={stats.data ? stats.data.selected_agent_count : placeholder}
              hint={`${providerBreakdown.length} with sessions`}
              variant="square"
            />
            <MetricTile
              label="Hook Health"
              value={hookSummary ? hookSummary.needs_attention : placeholder}
              hint={
                hookSummary
                  ? `${hookSummary.installed_ok} ok / ${hookSummary.not_installed} missing`
                  : "needs attention"
              }
              variant="square"
            />
            <MetricTile
              label="Activity"
              value={activityValue}
              hint={hours ? `${hours}h weighted score` : "all-time weighted score"}
              variant="square"
            />
            <MetricTile
              label="Workspaces"
              value={stats.data ? stats.data.all_workspace_count : placeholder}
              hint={allWorkspaces ? "all indexed paths" : "across providers"}
              variant="square"
            />
          </MetricGrid>
        </section>

        <StatsOverviewPanels
          isLoading={loading || activityLoading || sessions.isLoading}
          range={range}
          tableItems={usageTable.items}
          tableLabelColumn={usageTable.labelColumn}
          tableValueColumn={usageTable.valueColumn}
          timeline={activityTimeline}
        />
      </div>
    </ScrollArea>
  );
}
