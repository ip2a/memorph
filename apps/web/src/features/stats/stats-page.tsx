import { useState } from "react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import {
  Field,
  FieldContent,
  FieldGroup,
  FieldTitle,
} from "@/components/ui/field";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  ActivityTrend,
  InactivityPanel,
  ProviderPiePanel,
  RankingBoard,
} from "@/features/stats/stats-overview-panels";
import {
  type StatsWorkspaceScope,
  useStatsDashboard,
} from "@/features/stats/queries";
import { formatBytes } from "@/lib/format";
import type { StatsDashboardRange } from "@/lib/types";

const rangeLabels: Record<StatsDashboardRange, string> = {
  "7d": "近 7 天",
  "30d": "近 30 天",
  "90d": "近 90 天",
  all: "全部时间",
};

type OverviewMetric = {
  label: string;
  value: string;
  hint: string;
};

function StatsMetricRow({ label, value, hint }: OverviewMetric) {
  return (
    <Field orientation="horizontal" className="items-center py-1.5">
      <FieldContent className="gap-0">
        <FieldTitle className="text-sm">{label}</FieldTitle>
      </FieldContent>
      <p className="shrink-0 text-sm tabular-nums">
        <span className="font-medium">{value}</span>
        {hint ? <span className="text-muted-foreground"> · {hint}</span> : null}
      </p>
    </Field>
  );
}

export function StatsPage() {
  const [range, setRange] = useState<StatsDashboardRange>("30d");
  const [scope, setScope] = useState<StatsWorkspaceScope>("workspace");
  const { dashboard, meta, all } = useStatsDashboard(range, scope);

  if (meta.isLoading || dashboard.isLoading) return <PageSkeleton />;
  if (meta.error || dashboard.error) {
    return (
      <PageError
        title="统计加载失败"
        message={(meta.error ?? dashboard.error)?.message ?? "未知错误"}
      />
    );
  }
  if (!dashboard.data) {
    return <PageError title="暂无统计数据" message="请先选择一个工作空间。" />;
  }

  const data = dashboard.data;
  const period = rangeLabels[range];
  const metrics: OverviewMetric[] = [
    {
      label: "总会话",
      value: data.overview.total_sessions.toLocaleString(),
      hint: `累计 · ${data.overview.new_sessions} 个新增`,
    },
    {
      label: "活跃会话",
      value: data.overview.active_sessions.toLocaleString(),
      hint: period,
    },
    {
      label: "消息总量",
      value: data.overview.total_messages.toLocaleString(),
      hint: `活跃会话含 ${data.overview.active_session_messages.toLocaleString()} 条`,
    },
    {
      label: "数据占用",
      value: formatBytes(data.overview.total_size_bytes),
      hint: `长期未活跃 ${formatBytes(data.overview.stale_size_bytes)}`,
    },
    {
      label: "90 天以上未活跃",
      value: data.attention.inactive_over_90d.count.toLocaleString(),
      hint: `占用 ${formatBytes(data.attention.inactive_over_90d.size_bytes)}`,
    },
    {
      label: "大型会话",
      value: data.attention.large_sessions.count.toLocaleString(),
      hint: `占用 ${formatBytes(data.attention.large_sessions.size_bytes)}`,
    },
    {
      label: "内容较少",
      value: data.attention.short_sessions.count.toLocaleString(),
      hint: `占用 ${formatBytes(data.attention.short_sessions.size_bytes)}`,
    },
  ];

  return (
    <ScrollArea className="h-full" data-stats-page>
      <div className="flex flex-col gap-6 pb-6">
        <div className="flex flex-wrap items-center justify-end gap-2">
          <span className="text-xs text-muted-foreground">活跃指标范围</span>
          <Tabs
            value={scope}
            onValueChange={(value) => setScope(value as StatsWorkspaceScope)}
          >
            <TabsList>
              <TabsTrigger value="workspace">当前工作空间</TabsTrigger>
              <TabsTrigger value="all">全部工作空间</TabsTrigger>
            </TabsList>
          </Tabs>
          <Tabs
            value={range}
            onValueChange={(value) => setRange(value as StatsDashboardRange)}
          >
            <TabsList>
              <TabsTrigger value="7d">7 天</TabsTrigger>
              <TabsTrigger value="30d">30 天</TabsTrigger>
              <TabsTrigger value="90d">90 天</TabsTrigger>
              <TabsTrigger value="all">全部</TabsTrigger>
            </TabsList>
          </Tabs>
        </div>

        <section className="grid grid-cols-10 items-start gap-4">
          <div className="col-span-4 min-w-0" data-stats-overview>
            <FieldGroup className="gap-0 divide-y divide-border">
              {metrics.map((metric) => (
                <StatsMetricRow key={metric.label} {...metric} />
              ))}
            </FieldGroup>
          </div>

          <div className="col-span-3 min-w-0">
            <InactivityPanel
              data={data.attention}
              sessionSize={data.distributions.session_size}
            />
          </div>
          <div className="col-span-3 min-w-0">
            <ProviderPiePanel
              items={data.providers}
              messageCount={data.distributions.message_count}
            />
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
      </div>
    </ScrollArea>
  );
}
