import { useMemo, useState, type HTMLAttributes } from "react";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { SectionHeading } from "@/components/shared/section-heading";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SessionActivityChart } from "@/features/sessions/session-activity-chart";
import { ProviderPieChart, StatsRankBarChart } from "@/features/stats/stats-charts";
import { type StatsRange, type StatsWorkspaceScope, useStatsDashboard } from "@/features/stats/queries";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";

function DividerSection({
  children,
  className,
  title,
  ...props
}: HTMLAttributes<HTMLElement> & {
  title?: string;
}) {
  return (
    <section className={cn("flex flex-col gap-3 border-b pb-4", className)} {...props}>
      {title ? <strong className="text-sm font-medium">{title}</strong> : null}
      {children}
    </section>
  );
}

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

  const providerShare = useMemo(() => {
    const top = providerBreakdown.slice(0, 5);
    const rest = providerBreakdown.slice(5).reduce((sum, item) => sum + item.value, 0);
    const items = top.map((item) => ({
      id: item.id,
      label: item.label,
      value: item.value,
    }));
    if (rest > 0) {
      items.push({ id: "other", label: "Other", value: rest });
    }
    return items;
  }, [providerBreakdown]);

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
          <MetricGrid columns="auto" className="grid-cols-2 sm:grid-cols-3 xl:grid-cols-6">
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
              hint={`${hours}h weighted score`}
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

        <section className="grid gap-4 lg:grid-cols-2" data-stats-primary-charts>
          <Card size="sm">
            <CardHeader>
              <CardTitle>{allWorkspaces ? "Top Workspaces" : "Top Sessions"}</CardTitle>
              <CardDescription>按消息数量排名</CardDescription>
            </CardHeader>
            <CardContent>
              <StatsRankBarChart
                isLoading={loading || sessions.isLoading}
                items={rankBarItems}
                emptyLabel={allWorkspaces ? "暂无工作空间消息数据。" : "当前工作区暂无会话消息数据。"}
              />
            </CardContent>
          </Card>

          <Card size="sm">
            <CardHeader>
              <CardTitle>Provider Share</CardTitle>
              <CardDescription>会话分布占比</CardDescription>
            </CardHeader>
            <CardContent>
              <ProviderPieChart items={providerShare} emptyLabel="当前工作区暂无会话。" />
            </CardContent>
          </Card>
        </section>

        <DividerSection data-stats-activity-chart className="border-b-0 pb-0">
          <div className="flex flex-col gap-1">
            <strong className="text-sm font-medium">Activity Timeline</strong>
            <p className="text-xs text-muted-foreground">选定时间范围内的活动趋势</p>
          </div>
          <SessionActivityChart timeline={activityTimeline} isLoading={activityLoading} />
        </DividerSection>
      </div>
    </ScrollArea>
  );
}
