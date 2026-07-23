import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import {
  CartesianGrid,
  Cell,
  Label,
  Line,
  LineChart,
  Pie,
  PieChart,
  Sector,
  XAxis,
  YAxis,
} from "recharts";
import type { PieSectorShapeProps } from "recharts";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { workspaceName } from "@/components/shared/workspace-name";
import { SectionHeading } from "@/components/shared/section-heading";
import { Badge } from "@/components/ui/badge";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Empty, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatBytes, formatDateTime } from "@/lib/format";
import type {
  StatsBreakdownItem,
  StatsDashboard,
  StatsSessionItem,
} from "@/lib/types";

const chartConfig = {
  active_sessions: { label: "活跃会话", color: "var(--chart-1)" },
  new_sessions: { label: "新增会话", color: "var(--chart-2)" },
  active_session_messages: { label: "活跃消息", color: "var(--chart-3)" },
} satisfies ChartConfig;

type TrendKey = keyof typeof chartConfig;
type RankKey = "by_messages" | "by_size" | "recently_active";
type BreakdownKind = "provider" | "workspace";

function PanelEmpty() {
  return (
    <Empty className="min-h-32 border-none">
      <EmptyHeader>
        <EmptyTitle>暂无数据</EmptyTitle>
      </EmptyHeader>
    </Empty>
  );
}

function breakdownLabel(item: StatsBreakdownItem, kind: BreakdownKind) {
  if (kind === "workspace") return workspaceName(item.id);
  return item.id.split(/[\\/]/).filter(Boolean).at(-1) ?? item.id;
}

export function ActivityTrend({
  data,
  unknownMessageTimestamps,
}: {
  data: StatsDashboard["timeline"];
  unknownMessageTimestamps: number;
}) {
  const [metric, setMetric] = useState<TrendKey>("active_sessions");
  const spansYears =
    data.length > 1 &&
    new Date(data[0].start).getFullYear() !==
      new Date(data.at(-1)!.start).getFullYear();
  const points = data.map((point) => ({
    ...point,
    label: new Date(point.start).toLocaleDateString("zh-CN", {
      year: spansYears ? "numeric" : undefined,
      month: "numeric",
      day: "numeric",
    }),
  }));
  const hasSeries = points.some((point) => point[metric] > 0);
  const incompleteMessages =
    metric === "active_session_messages" && unknownMessageTimestamps > 0;

  return (
    <section className="flex min-w-0 flex-col gap-3">
      <SectionHeading
        variant="compact"
        title="使用趋势"
        actions={
          <Tabs
            value={metric}
            onValueChange={(value) => setMetric(value as TrendKey)}
          >
            <TabsList>
              <TabsTrigger value="active_sessions">活跃会话</TabsTrigger>
              <TabsTrigger value="new_sessions">新增会话</TabsTrigger>
              <TabsTrigger value="active_session_messages">
                活跃消息
              </TabsTrigger>
            </TabsList>
          </Tabs>
        }
      />
      {incompleteMessages ? (
        <p className="text-xs text-muted-foreground">
          有 {unknownMessageTimestamps.toLocaleString()}{" "}
          条消息缺少时间戳，趋势会展示其余可定位消息。
        </p>
      ) : null}
      {points.length && hasSeries ? (
        <ChartContainer config={chartConfig} className="h-72 w-full">
          <LineChart accessibilityLayer data={points}>
            <CartesianGrid vertical={false} strokeDasharray="4 4" />
            <XAxis dataKey="label" tickLine={false} axisLine={false} />
            <YAxis
              tickLine={false}
              axisLine={false}
              width={44}
              allowDecimals={false}
            />
            <ChartTooltip content={<ChartTooltipContent />} />
            <Line
              type="monotone"
              dataKey={metric}
              stroke={`var(--color-${metric})`}
              strokeWidth={2}
              dot={false}
            />
          </LineChart>
        </ChartContainer>
      ) : (
        <PanelEmpty />
      )}
    </section>
  );
}

const inactivityChartConfig = {
  value: { label: "会话" },
  active_7d: { label: "7 天内", color: "var(--chart-1)" },
  inactive_7_to_30d: { label: "7–30 天", color: "var(--chart-2)" },
  inactive_30_to_90d: { label: "30–90 天", color: "var(--chart-3)" },
  inactive_over_90d: { label: "90 天以上", color: "var(--chart-4)" },
  unknown: { label: "未知", color: "var(--chart-5)" },
} satisfies ChartConfig;

const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
] as const;

function pieSliceKey(id: string, index: number) {
  const safe = id.replace(/[^a-zA-Z0-9_-]/g, "_") || `item_${index}`;
  return `${safe}_${index}`;
}

function ActivePieShape({ isActive, ...props }: PieSectorShapeProps) {
  return (
    <Sector
      {...props}
      outerRadius={isActive ? props.outerRadius + 8 : props.outerRadius}
    />
  );
}

type PieSeries = {
  id: string;
  label: string;
  centerLabel?: string;
  config: ChartConfig;
  data: Array<{ id: string; value: number; fill: string }>;
};

function StatsPieChart({
  config,
  data,
  centerLabel = "会话",
}: {
  config: ChartConfig;
  data: Array<{ id: string; value: number; fill: string }>;
  centerLabel?: string;
}) {
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const total = data.reduce((sum, item) => sum + item.value, 0);
  const selected = selectedIndex == null ? null : (data[selectedIndex] ?? null);
  const selectedConfig = selected ? config[selected.id] : null;
  const displayValue = selected?.value ?? total;
  const displayLabel =
    selected == null
      ? centerLabel
      : typeof selectedConfig?.label === "string"
        ? selectedConfig.label
        : String(selected.id);

  if (!data.length) return <PanelEmpty />;

  return (
    <ChartContainer
      config={config}
      className="mx-auto aspect-square max-h-52 w-full"
    >
      <PieChart>
        <ChartTooltip
          content={<ChartTooltipContent nameKey="id" hideLabel />}
        />
        <Pie
          data={data}
          dataKey="value"
          nameKey="id"
          innerRadius={48}
          outerRadius={72}
          strokeWidth={2}
          isAnimationActive
          animationBegin={80}
          animationDuration={900}
          animationEasing="ease-out"
          shape={(props: PieSectorShapeProps) => (
            <ActivePieShape
              {...props}
              isActive={props.index === selectedIndex || props.isActive}
            />
          )}
          onMouseEnter={(_, index) => setSelectedIndex(index)}
          onClick={(_, index) => setSelectedIndex(index)}
        >
          {data.map((item) => (
            <Cell
              key={item.id}
              fill={item.fill}
              className="cursor-pointer outline-none"
            />
          ))}
          <Label
            content={({ viewBox }) => {
              if (!viewBox || !("cx" in viewBox) || !("cy" in viewBox))
                return null;
              return (
                <text
                  x={viewBox.cx}
                  y={viewBox.cy}
                  textAnchor="middle"
                  dominantBaseline="middle"
                >
                  <tspan
                    x={viewBox.cx}
                    y={(viewBox.cy ?? 0) - 6}
                    className="fill-foreground text-xl font-semibold tabular-nums"
                  >
                    {displayValue.toLocaleString()}
                  </tspan>
                  <tspan
                    x={viewBox.cx}
                    y={(viewBox.cy ?? 0) + 14}
                    className="fill-muted-foreground text-xs"
                  >
                    {displayLabel}
                  </tspan>
                </text>
              );
            }}
          />
        </Pie>
      </PieChart>
    </ChartContainer>
  );
}

function TabbedStatsPie({ series }: { series: PieSeries[] }) {
  const [tab, setTab] = useState(series[0]?.id ?? "");
  const active = series.find((item) => item.id === tab) ?? series[0];
  if (!active) return null;

  return (
    <section className="flex h-full min-w-0 flex-col gap-2">
      <Tabs value={active.id} onValueChange={setTab}>
        <TabsList className="w-full flex-wrap">
          {series.map((item) => (
            <TabsTrigger key={item.id} value={item.id} className="flex-1">
              {item.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <StatsPieChart
        key={active.id}
        config={active.config}
        data={active.data}
        centerLabel={active.centerLabel}
      />
    </section>
  );
}

function distributionPieSeries(
  id: string,
  label: string,
  items: StatsDashboard["distributions"]["session_size"],
  centerLabel: string,
): PieSeries {
  const top = items.filter((item) => item.count > 0);
  const config: ChartConfig = {
    value: { label: centerLabel },
    ...Object.fromEntries(
      top.map((item, index) => [
        item.key,
        { label: item.label, color: CHART_COLORS[index % CHART_COLORS.length] },
      ]),
    ),
  };
  return {
    id,
    label,
    centerLabel,
    config,
    data: top.map((item) => ({
      id: item.key,
      value: item.count,
      fill: `var(--color-${item.key})`,
    })),
  };
}

export function InactivityPanel({
  data,
  sessionSize,
}: {
  data: StatsDashboard["attention"];
  sessionSize: StatsDashboard["distributions"]["session_size"];
}) {
  const items = [
    { id: "active_7d" as const, value: data.active_7d.count },
    { id: "inactive_7_to_30d" as const, value: data.inactive_7_to_30d.count },
    { id: "inactive_30_to_90d" as const, value: data.inactive_30_to_90d.count },
    { id: "inactive_over_90d" as const, value: data.inactive_over_90d.count },
    { id: "unknown" as const, value: data.unknown.count },
  ].filter((item) => item.value > 0);

  return (
    <TabbedStatsPie
      series={[
        {
          id: "activity",
          label: "活跃状态",
          centerLabel: "会话",
          config: inactivityChartConfig,
          data: items.map((item) => ({
            ...item,
            fill: `var(--color-${item.id})`,
          })),
        },
        distributionPieSeries("session_size", "会话大小", sessionSize, "会话"),
      ]}
    />
  );
}

export function ProviderPiePanel({
  items,
  messageCount,
}: {
  items: StatsBreakdownItem[];
  messageCount: StatsDashboard["distributions"]["message_count"];
}) {
  const top = items.filter((item) => item.session_count > 0).slice(0, 8);
  const config: ChartConfig = {
    value: { label: "会话" },
    ...Object.fromEntries(
      top.map((item, index) => [
        pieSliceKey(item.id, index),
        {
          label: breakdownLabel(item, "provider"),
          color: CHART_COLORS[index % CHART_COLORS.length],
        },
      ]),
    ),
  };
  const pieData = top.map((item, index) => {
    const id = pieSliceKey(item.id, index);
    return { id, value: item.session_count, fill: `var(--color-${id})` };
  });

  return (
    <TabbedStatsPie
      series={[
        {
          id: "providers",
          label: "Agent",
          centerLabel: "会话",
          config,
          data: pieData,
        },
        distributionPieSeries(
          "message_count",
          "消息数量",
          messageCount,
          "会话",
        ),
      ]}
    />
  );
}

export function RankingBoard({
  sessions,
  providers,
  workspaces,
  all,
}: {
  sessions: StatsDashboard["top_sessions"];
  providers: StatsBreakdownItem[];
  workspaces: StatsBreakdownItem[];
  all: boolean;
}) {
  const [limit, setLimit] = useState<"5" | "10">("5");
  const [rank, setRank] = useState<RankKey>("by_messages");
  const [breakdownKind, setBreakdownKind] = useState<BreakdownKind>("provider");
  const topN = Number(limit);
  const effectiveKind: BreakdownKind = all ? breakdownKind : "provider";
  const breakdownItems = (effectiveKind === "workspace" ? workspaces : providers).slice(
    0,
    topN,
  );
  const sessionItems = sessions[rank].slice(0, topN);

  const topLimitTabs = (
    <Tabs
      value={limit}
      onValueChange={(value) => setLimit(value as "5" | "10")}
    >
      <TabsList>
        <TabsTrigger value="5">Top 5</TabsTrigger>
        <TabsTrigger value="10">Top 10</TabsTrigger>
      </TabsList>
    </Tabs>
  );

  return (
    <section className="grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-10">
      <div className="min-w-0 xl:col-span-7">
        <SectionHeading
          variant="compact"
          title="会话排行"
          actions={
            <>
              <Tabs
                value={rank}
                onValueChange={(value) => setRank(value as RankKey)}
              >
                <TabsList>
                  <TabsTrigger value="by_messages">消息最多</TabsTrigger>
                  <TabsTrigger value="by_size">占用最大</TabsTrigger>
                  <TabsTrigger value="recently_active">最近活跃</TabsTrigger>
                </TabsList>
              </Tabs>
              {topLimitTabs}
            </>
          }
        />
        <div className="flex flex-col divide-y divide-border">
          {sessionItems.length ? (
            <SessionList items={sessionItems} />
          ) : (
            <PanelEmpty />
          )}
        </div>
      </div>

      <div className="min-w-0 xl:col-span-3">
        <SectionHeading
          variant="compact"
          title={
            <Tabs
              value={effectiveKind}
              onValueChange={(value) => setBreakdownKind(value as BreakdownKind)}
            >
              <TabsList>
                <TabsTrigger value="provider">Agent 排行</TabsTrigger>
                <TabsTrigger value="workspace" disabled={!all}>
                  工作空间排行
                </TabsTrigger>
              </TabsList>
            </Tabs>
          }
          actions={topLimitTabs}
        />
        <div className="flex flex-col divide-y divide-border">
          {breakdownItems.length ? (
            <BreakdownRows items={breakdownItems} kind={effectiveKind} />
          ) : (
            <PanelEmpty />
          )}
        </div>
      </div>
    </section>
  );
}

function RankingRow({
  rank,
  title,
  leadingTags,
  trailingTags,
}: {
  rank: number;
  title: ReactNode;
  leadingTags?: ReactNode;
  trailingTags?: ReactNode;
}) {
  return (
    <div className="flex min-h-[4.5rem] gap-3 py-3">
      <span className="w-5 shrink-0 pt-0.5 text-sm tabular-nums text-muted-foreground">
        {rank}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="min-w-0 truncate text-sm leading-5 font-medium">{title}</div>
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 flex-wrap gap-2">{leadingTags}</div>
          <div className="flex shrink-0 flex-wrap justify-end gap-2">{trailingTags}</div>
        </div>
      </div>
    </div>
  );
}

function BreakdownRankingRow({
  rank,
  kind,
  item,
}: {
  rank: number;
  kind: BreakdownKind;
  item: StatsBreakdownItem;
}) {
  const tags = (
    <>
      <Badge variant="secondary">{item.session_count} 会话</Badge>
      <Badge variant="outline">
        {item.message_count.toLocaleString()} 消息
      </Badge>
      <Badge variant="outline">{formatBytes(item.size_bytes)}</Badge>
    </>
  );

  if (kind === "provider") {
    return (
      <div className="flex min-h-[4.5rem] gap-3 py-3">
        <span className="w-5 shrink-0 self-center text-sm tabular-nums text-muted-foreground">
          {rank}
        </span>
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <ProviderLogo providerId={item.id} size="sm" alt={item.id} />
            <span className="min-w-0 truncate text-base leading-tight font-semibold">
              {item.id}
            </span>
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-2">{tags}</div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-[4.5rem] gap-3 py-3">
      <span className="w-5 shrink-0 pt-0.5 text-sm tabular-nums text-muted-foreground">
        {rank}
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <div className="flex min-w-0 flex-1 flex-col justify-center gap-1">
          <div className="min-w-0 truncate text-sm leading-5 font-medium">
            {workspaceName(item.id)}
          </div>
          <PathText value={item.id} wrap="truncate" className="min-w-0 leading-5" />
        </div>
        <div className="flex shrink-0 flex-wrap justify-end gap-2">{tags}</div>
      </div>
    </div>
  );
}

function BreakdownRows({
  items,
  kind,
}: {
  items: StatsBreakdownItem[];
  kind: BreakdownKind;
}) {
  return (
    <>
      {items.map((item, index) => (
        <BreakdownRankingRow
          key={item.id}
          rank={index + 1}
          kind={kind}
          item={item}
        />
      ))}
    </>
  );
}

function SessionList({ items }: { items: StatsSessionItem[] }) {
  return (
    <>
      {items.map((item, index) => (
        <RankingRow
          key={`${item.provider_id}:${item.session_id}`}
          rank={index + 1}
          title={
            <Link
              className="block truncate hover:underline"
              to={`/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`}
            >
              {item.title}
            </Link>
          }
          leadingTags={
            <>
              <Badge variant="secondary">{item.provider_id}</Badge>
              <Badge variant="outline">
                {formatDateTime(item.last_active_at)}
              </Badge>
            </>
          }
          trailingTags={
            <>
              <Badge variant="secondary">
                {item.message_count?.toLocaleString() ?? "—"} 消息
              </Badge>
              <Badge variant="outline">{formatBytes(item.size_bytes)}</Badge>
            </>
          }
        />
      ))}
    </>
  );
}
