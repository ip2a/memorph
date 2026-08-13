import { useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Line,
  LineChart,
  Pie,
  PieChart,
  XAxis,
  YAxis,
} from "recharts";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { workspaceName } from "@/components/shared/workspace-name";
import { SectionHeading } from "@/components/shared/section-heading";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Empty, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatBytes, formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type {
  StatsBreakdownItem,
  StatsDashboard,
  StatsSessionItem,
} from "@/lib/types";

type TrendKey = "active_sessions" | "new_sessions" | "active_session_messages";
type RankKey = "by_messages" | "by_size" | "recently_active";
type BreakdownKind = "provider" | "workspace";
type DistributionChartKind = "donut" | "bar";

const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
] as const;

export const STATS_BAR_MAX_ROWS = 5;

function PanelEmpty() {
  const { t } = useI18n();
  return (
    <Empty className="min-h-32 border-none">
      <EmptyHeader>
        <EmptyTitle>{t("statsNoData")}</EmptyTitle>
      </EmptyHeader>
    </Empty>
  );
}

function breakdownLabel(item: StatsBreakdownItem, kind: BreakdownKind) {
  if (kind === "workspace") return workspaceName(item.id);
  return item.id.split(/[\\/]/).filter(Boolean).at(-1) ?? item.id;
}

function buildTrendPoints(
  data: StatsDashboard["timeline"],
  language: string,
) {
  const dateLocale = language === "zh" ? "zh-CN" : "en-US";
  const spansYears =
    data.length > 1 &&
    new Date(data[0].start).getFullYear() !==
      new Date(data.at(-1)!.start).getFullYear();
  return data.map((point) => ({
    ...point,
    label: new Date(point.start).toLocaleDateString(dateLocale, {
      year: spansYears ? "numeric" : undefined,
      month: "numeric",
      day: "numeric",
    }),
  }));
}

function ActivityTrendLineChart({
  points,
  metric,
  chartConfig,
}: {
  points: ReturnType<typeof buildTrendPoints>;
  metric: TrendKey;
  chartConfig: ChartConfig;
}) {
  const hasSeries = points.some((point) => point[metric] > 0);

  if (!points.length || !hasSeries) {
    return <PanelEmpty />;
  }

  return (
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
  );
}

export function ActivityTrend({
  data,
  unknownMessageTimestamps,
  splitMessagesTrend = false,
}: {
  data: StatsDashboard["timeline"];
  unknownMessageTimestamps: number;
  splitMessagesTrend?: boolean;
}) {
  const { t, language } = useI18n();
  const [sessionMetric, setSessionMetric] = useState<
    "active_sessions" | "new_sessions"
  >("active_sessions");
  const [combinedMetric, setCombinedMetric] = useState<TrendKey>("active_sessions");
  const chartConfig = useMemo(
    () =>
      ({
        active_sessions: {
          label: t("statsActiveSessions"),
          color: "var(--chart-1)",
        },
        new_sessions: { label: t("statsNewSessions"), color: "var(--chart-2)" },
        active_session_messages: {
          label: t("statsActiveMessages"),
          color: "var(--chart-3)",
        },
      }) satisfies ChartConfig,
    [t],
  );
  const points = useMemo(
    () => buildTrendPoints(data, language),
    [data, language],
  );

  if (splitMessagesTrend) {
    return (
      <div className="flex min-w-0 flex-col gap-6">
        <section className="flex min-w-0 flex-col gap-3">
          <SectionHeading
            variant="compact"
            title={t("statsUsageTrend")}
            actions={
              <Tabs
                value={sessionMetric}
                onValueChange={(value) =>
                  setSessionMetric(value as "active_sessions" | "new_sessions")
                }
              >
                <TabsList>
                  <TabsTrigger value="active_sessions">
                    {t("statsActiveSessions")}
                  </TabsTrigger>
                  <TabsTrigger value="new_sessions">
                    {t("statsNewSessions")}
                  </TabsTrigger>
                </TabsList>
              </Tabs>
            }
          />
          <ActivityTrendLineChart
            points={points}
            metric={sessionMetric}
            chartConfig={chartConfig}
          />
        </section>

        <section className="flex min-w-0 flex-col gap-3">
          <SectionHeading
            variant="compact"
            title={t("statsActiveMessages")}
          />
          <ActivityTrendLineChart
            points={points}
            metric="active_session_messages"
            chartConfig={chartConfig}
          />
        </section>
      </div>
    );
  }

  const incompleteCombinedMessages =
    combinedMetric === "active_session_messages" && unknownMessageTimestamps > 0;

  return (
    <section className="flex min-w-0 flex-col gap-3">
      <SectionHeading
        variant="compact"
        title={t("statsUsageTrend")}
        actions={
          <Tabs
            value={combinedMetric}
            onValueChange={(value) => setCombinedMetric(value as TrendKey)}
          >
            <TabsList>
              <TabsTrigger value="active_sessions">
                {t("statsActiveSessions")}
              </TabsTrigger>
              <TabsTrigger value="new_sessions">
                {t("statsNewSessions")}
              </TabsTrigger>
              <TabsTrigger value="active_session_messages">
                {t("statsActiveMessages")}
              </TabsTrigger>
            </TabsList>
          </Tabs>
        }
      />
      {incompleteCombinedMessages ? (
        <p className="text-xs text-muted-foreground">
          {t("statsTrendIncompleteTimestamps", {
            count: unknownMessageTimestamps.toLocaleString(),
          })}
        </p>
      ) : null}
      <ActivityTrendLineChart
        points={points}
        metric={combinedMetric}
        chartConfig={chartConfig}
      />
    </section>
  );
}

function barItemKey(id: string, index: number) {
  const safe = id.replace(/[^a-zA-Z0-9_-]/g, "_") || `item_${index}`;
  return `${safe}_${index}`;
}

type BarDatum = {
  id: string;
  value: number;
  fill: string;
  label?: ReactNode;
};

type BarSeries = {
  id: string;
  label: string;
  unitLabel?: string;
  chartKind?: DistributionChartKind;
  config: ChartConfig;
  data: BarDatum[];
};

function barLabel(item: BarDatum, config: ChartConfig) {
  if (item.label) return item.label;
  const entry = config[item.id];
  return typeof entry?.label === "string" ? entry.label : item.id;
}

function barFill(config: ChartConfig, id: string, index: number) {
  const entry = config[id];
  if (entry && "color" in entry && entry.color) return entry.color;
  return CHART_COLORS[index % CHART_COLORS.length];
}

function trimBarRows(
  data: BarDatum[],
  otherLabel: string,
  max = STATS_BAR_MAX_ROWS,
): BarDatum[] {
  if (data.length <= max) return data;
  const head = data.slice(0, max - 1);
  const tail = data.slice(max - 1);
  const otherValue = tail.reduce((sum, item) => sum + item.value, 0);
  return [
    ...head,
    {
      id: "__other__",
      value: otherValue,
      fill: CHART_COLORS[max - 1],
      label: otherLabel,
    },
  ];
}

function DistributionLegend({
  config,
  data,
}: {
  config: ChartConfig;
  data: BarDatum[];
}) {
  const total = data.reduce((sum, item) => sum + item.value, 0);

  return (
    <ul className="flex flex-col gap-1.5">
      {data.map((item, index) => {
        const percent = total > 0 ? (item.value / total) * 100 : 0;
        const color = item.fill || barFill(config, item.id, index);
        return (
          <li
            key={item.id}
            className="flex min-w-0 items-center justify-between gap-2 text-xs"
          >
            <span className="flex min-w-0 items-center gap-1.5">
              <span
                className="size-2 shrink-0 rounded-full"
                style={{ backgroundColor: color }}
              />
              <span className="min-w-0 truncate">{barLabel(item, config)}</span>
            </span>
            <span className="shrink-0 tabular-nums text-muted-foreground">
              {item.value.toLocaleString()}
              <span className="ml-1 text-[10px]">({percent.toFixed(0)}%)</span>
            </span>
          </li>
        );
      })}
    </ul>
  );
}

function DistributionDonutChart({
  config,
  data,
}: {
  config: ChartConfig;
  data: BarDatum[];
}) {
  if (!data.length) return <PanelEmpty />;

  return (
    <ChartContainer
      config={config}
      className="mx-auto aspect-square max-h-[168px] w-full"
    >
      <PieChart>
        <ChartTooltip content={<ChartTooltipContent hideLabel nameKey="id" />} />
        <Pie
          data={data}
          dataKey="value"
          nameKey="id"
          innerRadius="58%"
          outerRadius="86%"
          strokeWidth={2}
          paddingAngle={1}
        >
          {data.map((item, index) => (
            <Cell
              key={item.id}
              fill={item.fill || barFill(config, item.id, index)}
            />
          ))}
        </Pie>
      </PieChart>
    </ChartContainer>
  );
}

function DistributionBarChart({
  config,
  data,
}: {
  config: ChartConfig;
  data: BarDatum[];
}) {
  const chartData = useMemo(
    () =>
      data.map((item) => ({
        ...item,
        name:
          typeof barLabel(item, config) === "string"
            ? (barLabel(item, config) as string)
            : item.id,
      })),
    [config, data],
  );
  const chartHeight = Math.max(140, chartData.length * 34);

  if (!chartData.length) return <PanelEmpty />;

  return (
    <ChartContainer
      config={config}
      className="w-full"
      style={{ height: chartHeight }}
    >
      <BarChart
        accessibilityLayer
        data={chartData}
        layout="vertical"
        margin={{ left: 4, right: 8, top: 0, bottom: 0 }}
      >
        <CartesianGrid horizontal={false} strokeDasharray="3 3" />
        <XAxis type="number" hide allowDecimals={false} />
        <YAxis
          type="category"
          dataKey="name"
          width={88}
          tickLine={false}
          axisLine={false}
          tick={{ fontSize: 11 }}
        />
        <ChartTooltip content={<ChartTooltipContent hideLabel />} />
        <Bar dataKey="value" radius={[0, 4, 4, 0]}>
          {chartData.map((item, index) => (
            <Cell
              key={item.id}
              fill={item.fill || barFill(config, item.id, index)}
            />
          ))}
        </Bar>
      </BarChart>
    </ChartContainer>
  );
}

function DistributionCard({ series }: { series: BarSeries[] }) {
  const { t } = useI18n();
  const [view, setView] = useState(series[0]?.id ?? "");
  const active = series.find((item) => item.id === view) ?? series[0];
  if (!active) return null;

  const resolvedUnitLabel = active.unitLabel ?? t("statsSessionsUnit");
  const rows = trimBarRows(active.data, t("statsOther"));
  const total = rows.reduce((sum, item) => sum + item.value, 0);

  return (
    <div className="flex h-full min-w-0 flex-col border-b border-border pb-4">
      <div className="flex flex-wrap items-center justify-between gap-2 pb-2">
        <CardDescription>
          {t("statsBarTotal", {
            count: total.toLocaleString(),
            unit: resolvedUnitLabel,
          })}
        </CardDescription>
        <Tabs
          value={view}
          onValueChange={setView}
          className="ml-auto max-w-full"
        >
          <TabsList className="max-w-full flex-wrap">
            {series.map((item) => (
              <TabsTrigger key={item.id} value={item.id}>
                {item.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <div className="flex min-h-[180px] flex-1 flex-col gap-3 pt-3 sm:flex-row sm:items-center">
        {total > 0 ? (
          <>
            <div className="min-w-0 flex-1">
              <DistributionDonutChart config={active.config} data={rows} />
            </div>
            <div className="min-w-0 flex-1">
              <DistributionLegend config={active.config} data={rows} />
            </div>
          </>
        ) : (
          <PanelEmpty />
        )}
      </div>
    </div>
  );
}

export type StatsOverviewMetric = {
  label: string;
  value: string;
  hint: string;
};

export function StatsOverviewPanel({
  metrics,
}: {
  metrics: StatsOverviewMetric[];
}) {
  return (
    <section className="flex h-full min-w-0 flex-col py-1">
      <div className="flex flex-col divide-y divide-border">
        {metrics.map((metric) => (
          <div
            key={metric.label}
            className="flex items-center justify-between gap-3 py-1.5 text-sm"
          >
            <span className="shrink-0">{metric.label}</span>
            <p
              className="min-w-0 truncate text-right tabular-nums"
              title={metric.hint ? `${metric.value} · ${metric.hint}` : metric.value}
            >
              <span className="font-medium">{metric.value}</span>
              {metric.hint ? (
                <span className="text-muted-foreground"> · {metric.hint}</span>
              ) : null}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

function distributionBarSeries(
  id: string,
  label: string,
  items: StatsDashboard["distributions"]["session_size"],
  unitLabel: string,
): BarSeries {
  const top = items.filter((item) => item.count > 0);
  const config: ChartConfig = {
    value: { label: unitLabel },
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
    unitLabel,
    chartKind: "bar",
    config,
    data: top.map((item, index) => ({
      id: item.key,
      value: item.count,
      fill: barFill(config, item.key, index),
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
  const { t } = useI18n();
  const sessionsUnit = t("statsSessionsUnit");
  const inactivityChartConfig = useMemo(
    () =>
      ({
        value: { label: sessionsUnit },
        active_7d: { label: t("statsActive7d"), color: "var(--chart-1)" },
        inactive_7_to_30d: {
          label: t("statsInactive7to30d"),
          color: "var(--chart-2)",
        },
        inactive_30_to_90d: {
          label: t("statsInactive30to90d"),
          color: "var(--chart-3)",
        },
        inactive_over_90d: {
          label: t("statsInactiveOver90dChart"),
          color: "var(--chart-4)",
        },
        unknown: { label: t("statsUnknown"), color: "var(--chart-5)" },
      }) satisfies ChartConfig,
    [sessionsUnit, t],
  );
  const items = [
    { id: "active_7d" as const, value: data.active_7d.count },
    { id: "inactive_7_to_30d" as const, value: data.inactive_7_to_30d.count },
    { id: "inactive_30_to_90d" as const, value: data.inactive_30_to_90d.count },
    { id: "inactive_over_90d" as const, value: data.inactive_over_90d.count },
    { id: "unknown" as const, value: data.unknown.count },
  ].filter((item) => item.value > 0);

  return (
    <DistributionCard
      series={[
        {
          id: "activity",
          label: t("statsActivityStatus"),
          unitLabel: sessionsUnit,
          chartKind: "donut",
          config: inactivityChartConfig,
          data: items.map((item, index) => ({
            ...item,
            fill: barFill(inactivityChartConfig, item.id, index),
          })),
        },
        distributionBarSeries(
          "session_size",
          t("statsSessionSize"),
          sessionSize,
          sessionsUnit,
        ),
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
  const { t } = useI18n();
  const sessionsUnit = t("statsSessionsUnit");
  const top = items.filter((item) => item.session_count > 0);
  const config: ChartConfig = {
    value: { label: sessionsUnit },
    ...Object.fromEntries(
      top.map((item, index) => [
        barItemKey(item.id, index),
        {
          label: breakdownLabel(item, "provider"),
          color: CHART_COLORS[index % CHART_COLORS.length],
        },
      ]),
    ),
  };
  const providerData = top.map((item, index) => {
    const id = barItemKey(item.id, index);
    return {
      id,
      value: item.session_count,
      fill: barFill(config, id, index),
      label: (
        <span className="flex min-w-0 items-center gap-1.5">
          <ProviderLogo providerId={item.id} size="xs" alt={item.id} />
          <span className="truncate">{breakdownLabel(item, "provider")}</span>
        </span>
      ),
    };
  });

  return (
    <DistributionCard
      series={[
        {
          id: "providers",
          label: t("statsAgent"),
          unitLabel: sessionsUnit,
          chartKind: "donut",
          config,
          data: providerData,
        },
        distributionBarSeries(
          "message_count",
          t("statsMessageCount"),
          messageCount,
          sessionsUnit,
        ),
      ]}
    />
  );
}

export function StatsCompositionPanel({
  data,
}: {
  data: StatsDashboard;
}) {
  const { t } = useI18n();
  const sessionsUnit = t("statsSessionsUnit");
  const activityConfig = useMemo(
    () => ({
      value: { label: sessionsUnit },
      active_7d: { label: t("statsActive7d"), color: "var(--chart-1)" },
      inactive_7_to_30d: { label: t("statsInactive7to30d"), color: "var(--chart-2)" },
      inactive_30_to_90d: { label: t("statsInactive30to90d"), color: "var(--chart-3)" },
      inactive_over_90d: { label: t("statsInactiveOver90dChart"), color: "var(--chart-4)" },
      unknown: { label: t("statsUnknown"), color: "var(--chart-5)" },
    }) satisfies ChartConfig,
    [sessionsUnit, t],
  );
  const providerItems = data.providers.filter((item) => item.session_count > 0);
  const providerConfig = {
    value: { label: sessionsUnit },
    ...Object.fromEntries(
      providerItems.map((item, index) => [
        barItemKey(item.id, index),
        { label: breakdownLabel(item, "provider"), color: CHART_COLORS[index % CHART_COLORS.length] },
      ]),
    ),
  } satisfies ChartConfig;
  const activityItems = [
    { id: "active_7d", value: data.attention.active_7d.count },
    { id: "inactive_7_to_30d", value: data.attention.inactive_7_to_30d.count },
    { id: "inactive_30_to_90d", value: data.attention.inactive_30_to_90d.count },
    { id: "inactive_over_90d", value: data.attention.inactive_over_90d.count },
    { id: "unknown", value: data.attention.unknown.count },
  ].filter((item) => item.value > 0);

  return (
    <DistributionCard
      series={[
        { id: "activity", label: t("statsActivityStatus"), unitLabel: sessionsUnit, config: activityConfig, data: activityItems.map((item, index) => ({ ...item, fill: barFill(activityConfig, item.id, index) })) },
        distributionBarSeries("session_size", t("statsSessionSize"), data.distributions.session_size, sessionsUnit),
        { id: "providers", label: t("statsAgent"), unitLabel: sessionsUnit, config: providerConfig, data: providerItems.map((item, index) => { const id = barItemKey(item.id, index); return { id, value: item.session_count, fill: barFill(providerConfig, id, index), label: <span className="flex min-w-0 items-center gap-1.5"><ProviderLogo providerId={item.id} size="xs" alt={item.id} /><span className="truncate">{breakdownLabel(item, "provider")}</span></span> }; }) },
        distributionBarSeries("message_count", t("statsMessageCount"), data.distributions.message_count, sessionsUnit),
      ]}
    />
  );
}

export function StatsInsightsSection({
  data,
  all,
}: {
  data: StatsDashboard;
  all: boolean;
}) {
  const { t } = useI18n();

  return (
    <div className="flex min-w-0 flex-col gap-8 pb-2">
      <section className="flex min-w-0 flex-col gap-3">
        <SectionHeading variant="compact" title={t("statsComposition")} />
        <StatsCompositionPanel data={data} />
      </section>

      <RankingBoard
        sessions={data.top_sessions}
        providers={data.providers}
        workspaces={data.workspaces}
        all={all}
      />
    </div>
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
  const { t } = useI18n();
  type LeftView = RankKey | "workspace";
  const [view, setView] = useState<LeftView>("by_messages");
  const topN = 5;
  const isWorkspace = view === "workspace";
  const sessionItems = !isWorkspace ? sessions[view as RankKey].slice(0, topN) : [];
  const workspaceItems = isWorkspace ? workspaces.slice(0, topN) : [];
  const providerItems = providers.slice(0, topN);

  return (
    <section className="@container/stats-ranking flex min-w-0 flex-col gap-4">
      <SectionHeading
        variant="compact"
        title={t("statsRankings")}
        actions={
          <Tabs
            value={view}
            onValueChange={(value) => setView(value as LeftView)}
            className="max-w-full"
          >
            <TabsList className="max-w-full flex-wrap justify-end">
              <TabsTrigger value="by_messages">{t("statsByMessages")}</TabsTrigger>
              <TabsTrigger value="by_size">{t("statsBySize")}</TabsTrigger>
              <TabsTrigger value="recently_active">
                {t("statsRecentlyActive")}
              </TabsTrigger>
              <TabsTrigger value="workspace" disabled={!all}>
                {t("statsWorkspaceRanking")}
              </TabsTrigger>
            </TabsList>
          </Tabs>
        }
      />

      <div className="grid min-w-0 grid-cols-1 items-start gap-4 @xl/stats-ranking:grid-cols-[minmax(0,1fr)_minmax(0,1.35fr)]">
        <div className="min-w-0 border-b border-border pb-2">
          <div className="grid min-w-0 divide-y divide-border overflow-hidden">
            {providerItems.length ? (
              <BreakdownRows items={providerItems} kind="provider" />
            ) : (
              <PanelEmpty />
            )}
          </div>
        </div>

        <div className="min-w-0 border-b border-border pb-2">
          <div className="grid min-w-0 divide-y divide-border overflow-hidden">
            {isWorkspace ? (
              workspaceItems.length ? (
                <BreakdownRows items={workspaceItems} kind="workspace" />
              ) : (
                <PanelEmpty />
              )
            ) : sessionItems.length ? (
              <SessionList items={sessionItems} />
            ) : (
              <PanelEmpty />
            )}
          </div>
        </div>
      </div>
    </section>
  );
}

function BreakdownRankingRow({
  kind,
  item,
}: {
  kind: BreakdownKind;
  item: StatsBreakdownItem;
}) {
  const { t } = useI18n();
  const title =
    kind === "provider" ? item.id : workspaceName(item.id);

  return (
    <article className="grid h-20 min-w-0 items-center overflow-hidden px-4 py-2.5 hover:bg-muted/60">
      <div className="min-w-0 overflow-hidden">
        {kind === "provider" ? (
          <div className="flex min-w-0 items-center gap-2">
            <ProviderLogo providerId={item.id} size="sm" alt={item.id} />
            <span className="min-w-0 truncate font-semibold" title={title}>
              {title}
            </span>
          </div>
        ) : (
          <span className="block min-w-0 truncate font-semibold" title={title}>
            {title}
          </span>
        )}
        <div className="flex min-w-0 items-center gap-2 overflow-hidden whitespace-nowrap font-mono text-xs text-muted-foreground">
          {kind === "workspace" ? (
            <Badge variant="outline" className="max-w-full font-mono">
              <span className="truncate">{item.id}</span>
            </Badge>
          ) : null}
          <span className="shrink-0">
            {t("statsSessionCount", { count: item.session_count })}
          </span>
          <span className="shrink-0">
            {t("statsMessagesCount", {
              count: item.message_count.toLocaleString(),
            })}
          </span>
          <span className="shrink-0">{formatBytes(item.size_bytes)}</span>
        </div>
      </div>
    </article>
  );
}

function StatsSessionRow({ item }: { item: StatsSessionItem }) {
  const { t } = useI18n();
  const detailHref = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;

  return (
    <article className="grid h-20 min-w-0 items-center overflow-hidden px-4 py-2.5 hover:bg-muted/60">
      <div className="min-w-0 overflow-hidden">
        <Link
          to={detailHref}
          className="block min-w-0 truncate font-semibold hover:underline"
          title={item.title}
        >
          {item.title}
        </Link>
        <div className="flex min-w-0 items-center gap-2 overflow-hidden whitespace-nowrap font-mono text-xs text-muted-foreground">
          <Badge variant="outline" className="max-w-full font-mono">
            <span className="truncate">{item.provider_id}</span>
          </Badge>
          <span className="shrink-0">{formatDateTime(item.last_active_at)}</span>
          <span className="shrink-0">
            {t("statsMessagesCount", {
              count: item.message_count?.toLocaleString() ?? "—",
            })}
          </span>
          <span className="shrink-0">{formatBytes(item.size_bytes)}</span>
        </div>
      </div>
    </article>
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
      {items.map((item) => (
        <BreakdownRankingRow key={item.id} kind={kind} item={item} />
      ))}
    </>
  );
}

function SessionList({ items }: { items: StatsSessionItem[] }) {
  return (
    <>
      {items.map((item) => (
        <StatsSessionRow
          key={`${item.provider_id}:${item.session_id}`}
          item={item}
        />
      ))}
    </>
  );
}
