import { useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
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
import { Progress } from "@/components/ui/progress";
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

const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
] as const;

export const STATS_BAR_MAX_ROWS = 4;

const statsPanelTabListClassName =
  "h-9 max-w-full flex-nowrap overflow-x-auto";
const rankingTabListClassName = "max-w-full shrink-0 flex-nowrap overflow-x-auto";
const rankingTabTriggerClassName = "flex-none px-2 text-xs";
const statsPanelBodyClassName = "flex min-w-0 flex-col gap-3 py-1";
const statsPanelRowsClassName = "flex flex-col divide-y divide-border";
const statsPanelRowClassName = "flex min-w-0 flex-col gap-1.5 py-1.5";
const statsPanelRowHeaderClassName =
  "flex items-center justify-between gap-2 text-xs";

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

export function ActivityTrend({
  data,
  unknownMessageTimestamps,
}: {
  data: StatsDashboard["timeline"];
  unknownMessageTimestamps: number;
}) {
  const { t, language } = useI18n();
  const [metric, setMetric] = useState<TrendKey>("active_sessions");
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
  const dateLocale = language === "zh" ? "zh-CN" : "en-US";
  const spansYears =
    data.length > 1 &&
    new Date(data[0].start).getFullYear() !==
      new Date(data.at(-1)!.start).getFullYear();
  const points = data.map((point) => ({
    ...point,
    label: new Date(point.start).toLocaleDateString(dateLocale, {
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
        title={t("statsUsageTrend")}
        actions={
          <Tabs
            value={metric}
            onValueChange={(value) => setMetric(value as TrendKey)}
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
      {incompleteMessages ? (
        <p className="text-xs text-muted-foreground">
          {t("statsTrendIncompleteTimestamps", {
            count: unknownMessageTimestamps.toLocaleString(),
          })}
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

function StatsBarList({
  config,
  data,
  unitLabel,
  maxRows = STATS_BAR_MAX_ROWS,
}: {
  config: ChartConfig;
  data: BarDatum[];
  unitLabel?: string;
  maxRows?: number;
}) {
  const { t } = useI18n();
  const resolvedUnitLabel = unitLabel ?? t("statsSessionsUnit");
  const total = data.reduce((sum, item) => sum + item.value, 0);
  if (!data.length) return <PanelEmpty />;
  const rows = trimBarRows(data, t("statsOther"), maxRows);

  return (
    <div className={statsPanelBodyClassName}>
      <p className="text-xs leading-none text-muted-foreground">
        {t("statsBarTotal", {
          count: total.toLocaleString(),
          unit: resolvedUnitLabel,
        })}
      </p>
      <div className={statsPanelRowsClassName}>
        {rows.map((item, index) => {
          const percent = total > 0 ? (item.value / total) * 100 : 0;
          const color = item.fill || barFill(config, item.id, index);
          return (
            <div key={item.id} className={statsPanelRowClassName}>
              <div className={statsPanelRowHeaderClassName}>
                <span className="min-w-0 max-w-[45%] truncate">
                  {barLabel(item, config)}
                </span>
                <span className="min-w-0 flex-1 truncate text-right tabular-nums text-muted-foreground">
                  {item.value.toLocaleString()}
                  <span className="ml-1 text-[10px]">({percent.toFixed(0)}%)</span>
                </span>
              </div>
              <Progress
                value={percent}
                className="h-1.5 bg-muted/60 [&_[data-slot=progress-indicator]]:rounded-full [&_[data-slot=progress-indicator]]:bg-[var(--bar-color)]"
                style={{ "--bar-color": color } as React.CSSProperties}
              />
            </div>
          );
        })}
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
      <div className={statsPanelRowsClassName}>
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

function TabbedStatsBars({ series }: { series: BarSeries[] }) {
  const [tab, setTab] = useState(series[0]?.id ?? "");
  const active = series.find((item) => item.id === tab) ?? series[0];
  if (!active) return null;

  return (
    <section className="flex h-full min-w-0 flex-col gap-2">
      <Tabs value={active.id} onValueChange={setTab}>
        <TabsList className={statsPanelTabListClassName}>
          {series.map((item) => (
            <TabsTrigger key={item.id} value={item.id} className="flex-1">
              {item.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <StatsBarList
        key={active.id}
        config={active.config}
        data={active.data}
        unitLabel={active.unitLabel}
      />
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
    <TabbedStatsBars
      series={[
        {
          id: "activity",
          label: t("statsActivityStatus"),
          unitLabel: sessionsUnit,
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
    <TabbedStatsBars
      series={[
        {
          id: "providers",
          label: t("statsAgent"),
          unitLabel: sessionsUnit,
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
      <TabsList className={rankingTabListClassName}>
        <TabsTrigger value="5" className={rankingTabTriggerClassName}>
          Top 5
        </TabsTrigger>
        <TabsTrigger value="10" className={rankingTabTriggerClassName}>
          Top 10
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );

  const rankingHeadingClassName =
    "grid-cols-[minmax(0,1fr)_auto] items-center gap-2 border-b pb-2";
  const rankingActionsClassName =
    "flex min-w-0 flex-nowrap items-center justify-end gap-2 overflow-x-auto";

  return (
    <section className="@container/stats-ranking grid min-w-0 grid-cols-10 items-start gap-4">
      <div className="col-span-7 min-w-0">
        <SectionHeading
          variant="compact"
          className={rankingHeadingClassName}
          title={t("statsSessionRanking")}
          actionsProps={{ className: rankingActionsClassName }}
          actions={
            <>
              <Tabs
                value={rank}
                onValueChange={(value) => setRank(value as RankKey)}
              >
                <TabsList className={rankingTabListClassName}>
                  <TabsTrigger value="by_messages" className={rankingTabTriggerClassName}>
                    {t("statsByMessages")}
                  </TabsTrigger>
                  <TabsTrigger value="by_size" className={rankingTabTriggerClassName}>
                    {t("statsBySize")}
                  </TabsTrigger>
                  <TabsTrigger value="recently_active" className={rankingTabTriggerClassName}>
                    {t("statsRecentlyActive")}
                  </TabsTrigger>
                </TabsList>
              </Tabs>
              {topLimitTabs}
            </>
          }
        />
        <div className="grid min-w-0 divide-y divide-border overflow-hidden">
          {sessionItems.length ? (
            <SessionList items={sessionItems} />
          ) : (
            <PanelEmpty />
          )}
        </div>
      </div>

      <div className="col-span-3 min-w-0">
        <SectionHeading
          variant="compact"
          className={rankingHeadingClassName}
          title={
            <Tabs
              value={effectiveKind}
              onValueChange={(value) => setBreakdownKind(value as BreakdownKind)}
            >
              <TabsList className={rankingTabListClassName}>
                <TabsTrigger value="provider" className={rankingTabTriggerClassName}>
                  {t("statsAgentRanking")}
                </TabsTrigger>
                <TabsTrigger value="workspace" disabled={!all} className={rankingTabTriggerClassName}>
                  {t("statsWorkspaceRanking")}
                </TabsTrigger>
              </TabsList>
            </Tabs>
          }
          actionsProps={{ className: rankingActionsClassName }}
          actions={topLimitTabs}
        />
        <div className="grid min-w-0 divide-y divide-border overflow-hidden">
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
    <article className="grid min-h-14 min-w-0 py-2.5 hover:bg-muted/60">
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
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
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
    <article className="grid min-h-14 min-w-0 py-2.5 hover:bg-muted/60">
      <div className="min-w-0 overflow-hidden">
        <Link
          to={detailHref}
          className="block min-w-0 truncate font-semibold hover:underline"
          title={item.title}
        >
          {item.title}
        </Link>
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
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
