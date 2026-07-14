import { useMemo } from "react";
import { BarChart3Icon } from "lucide-react";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { StatsRange } from "@/features/stats/queries";
import {
  buildActivityLineData,
  buildActivityCoverageBlocks,
  type ActivityCoverageBlock,
  type UsageTableItem,
} from "@/features/stats/stats-activity-model";
import type { SessionActivityTimeline } from "@/lib/types";
import { cn } from "@/lib/utils";

const activityLineConfig = {
  activity: {
    label: "Activity",
    color: "var(--chart-1)",
  },
} satisfies ChartConfig;

function rangeLabel(range: StatsRange) {
  if (range === "all") return "all";
  return range;
}

function formatAxisValue(value: number) {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

export function StatsUsageTable({
  className,
  emptyLabel = "暂无数据",
  isLoading,
  items,
  labelColumn = "Provider",
  valueColumn = "Sessions",
}: {
  className?: string;
  emptyLabel?: string;
  isLoading?: boolean;
  items: UsageTableItem[];
  labelColumn?: string;
  valueColumn?: string;
}) {
  const total = useMemo(() => items.reduce((sum, item) => sum + item.value, 0), [items]);

  if (isLoading) {
    return (
      <div className={cn("flex flex-col gap-2", className)}>
        {Array.from({ length: 6 }).map((_, index) => (
          <Skeleton key={index} className="h-8 w-full" />
        ))}
      </div>
    );
  }

  if (!items.length) {
    return (
      <div className={cn("flex min-h-48 items-center justify-center text-sm text-muted-foreground", className)}>
        {emptyLabel}
      </div>
    );
  }

  return (
    <Table className={cn("table-fixed", className)}>
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead className="h-8 px-3 text-xs font-medium text-muted-foreground">{labelColumn}</TableHead>
          <TableHead className="h-8 w-24 px-3 text-right text-xs font-medium text-muted-foreground">
            {valueColumn}
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {items.map((item) => (
          <TableRow key={item.id} className="hover:bg-transparent">
            <TableCell className="max-w-0 truncate px-3 py-2 font-mono text-xs" title={item.label}>
              {item.label}
            </TableCell>
            <TableCell className="px-3 py-2 text-right font-mono text-xs tabular-nums">{item.value}</TableCell>
          </TableRow>
        ))}
      </TableBody>
      <TableFooter>
        <TableRow className="hover:bg-transparent">
          <TableCell className="px-3 py-2 text-xs font-medium">Total</TableCell>
          <TableCell className="px-3 py-2 text-right font-mono text-xs font-medium tabular-nums">{total}</TableCell>
        </TableRow>
      </TableFooter>
    </Table>
  );
}

export function StatsActivityCoverage({
  blocks,
  className,
  isLoading,
  range,
}: {
  blocks: ActivityCoverageBlock[];
  className?: string;
  isLoading?: boolean;
  range: StatsRange;
}) {
  const coveragePercent = useMemo(() => {
    if (!blocks.length) return 0;
    const activeCount = blocks.filter((block) => block.active).length;
    return (activeCount / blocks.length) * 100;
  }, [blocks]);

  if (isLoading) {
    return (
      <div className={cn("flex flex-col gap-3", className)}>
        <Skeleton className="h-4 w-40" />
        <Skeleton className="h-10 w-full" />
      </div>
    );
  }

  if (!blocks.length) {
    return (
      <div className={cn("flex min-h-24 items-center justify-center text-sm text-muted-foreground", className)}>
        暂无活动数据
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col gap-3", className)} data-stats-activity-coverage>
      <p className="font-mono text-xs text-foreground">
        Active intervals ({rangeLabel(range)}) {coveragePercent.toFixed(1)}%
      </p>
      <div className="flex items-end gap-1">
        {blocks.map((block) => (
          <div
            key={block.id}
            className={cn(
              "h-8 min-w-0 flex-1 rounded-sm",
              block.active ? "bg-chart-1" : "bg-muted",
            )}
            title={block.active ? "active" : "idle"}
          />
        ))}
      </div>
    </div>
  );
}

export function StatsActivityLineChart({
  className,
  isLoading,
  range,
  timeline,
}: {
  className?: string;
  isLoading?: boolean;
  range: StatsRange;
  timeline: SessionActivityTimeline | null | undefined;
}) {
  const chartData = useMemo(() => buildActivityLineData(timeline), [timeline]);
  const peak = useMemo(() => Math.max(...chartData.map((point) => point.activity), 0), [chartData]);
  const median = useMemo(() => {
    if (!chartData.length) return 0;
    const sorted = [...chartData.map((point) => point.activity)].sort((left, right) => left - right);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
  }, [chartData]);

  if (isLoading) {
    return (
      <div className={cn("flex flex-col gap-3", className)}>
        <Skeleton className="h-4 w-44" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (!chartData.length) {
    return (
      <div className={cn("flex min-h-40 items-center justify-center text-sm text-muted-foreground", className)}>
        暂无活动趋势
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col gap-3", className)} data-stats-activity-line>
      <p className="font-mono text-xs text-foreground">
        Activity ({rangeLabel(range)}) {formatAxisValue(median)} p50
      </p>
      <ChartContainer
        config={activityLineConfig}
        className="aspect-auto h-40 w-full"
        initialDimension={{ width: 480, height: 160 }}
      >
        <LineChart accessibilityLayer data={chartData} margin={{ left: 0, right: 8, top: 4, bottom: 0 }}>
          <CartesianGrid vertical={false} strokeDasharray="4 4" />
          <XAxis
            dataKey="label"
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            minTickGap={24}
            tick={{ fontSize: 11 }}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            width={40}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => formatAxisValue(Number(value))}
            domain={[0, Math.max(peak, 1)]}
          />
          <ChartTooltip
            cursor={false}
            content={<ChartTooltipContent hideLabel indicator="line" nameKey="label" />}
          />
          <Line
            type="monotone"
            dataKey="activity"
            stroke="var(--color-activity)"
            strokeWidth={2}
            dot={false}
            activeDot={{ r: 3 }}
          />
        </LineChart>
      </ChartContainer>
    </div>
  );
}

export function StatsOverviewPanels({
  isLoading,
  range,
  tableItems,
  tableLabelColumn,
  tableValueColumn,
  timeline,
}: {
  isLoading?: boolean;
  range: StatsRange;
  tableItems: UsageTableItem[];
  tableLabelColumn?: string;
  tableValueColumn?: string;
  timeline: SessionActivityTimeline | null | undefined;
}) {
  const activityBlocks = useMemo(() => buildActivityCoverageBlocks(timeline), [timeline]);

  return (
    <section className="grid gap-4 lg:grid-cols-3" data-stats-overview-panels>
      <Card size="sm" className="lg:col-span-1">
        <CardHeader className="border-b pb-3">
          <CardTitle className="flex items-center gap-2 font-heading text-base">
            <BarChart3Icon data-icon="inline-start" />
            Usage ranking
          </CardTitle>
        </CardHeader>
        <CardContent className="pt-0">
          <StatsUsageTable
            isLoading={isLoading}
            items={tableItems}
            labelColumn={tableLabelColumn}
            valueColumn={tableValueColumn}
            className="border-0"
          />
        </CardContent>
      </Card>

      <div className="flex flex-col gap-4 lg:col-span-2">
        <Card size="sm">
          <CardContent>
            <StatsActivityCoverage blocks={activityBlocks} isLoading={isLoading} range={range} />
          </CardContent>
        </Card>

        <Card size="sm">
          <CardContent>
            <StatsActivityLineChart isLoading={isLoading} range={range} timeline={timeline} />
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
