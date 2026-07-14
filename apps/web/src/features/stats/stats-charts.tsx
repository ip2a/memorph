import { useMemo } from "react";
import { Bar, BarChart, Cell, Pie, PieChart, XAxis, YAxis } from "recharts";
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

export type BarChartItem = {
  id: string;
  label: string;
  value: number;
};

export type PieChartItem = {
  id: string;
  label: string;
  value: number;
};

const CHART_PALETTE = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
] as const;

function chartColor(index: number) {
  return CHART_PALETTE[index % CHART_PALETTE.length];
}

const rankBarConfig = {
  value: {
    label: "Messages",
    color: "var(--chart-1)",
  },
} satisfies ChartConfig;

function buildPieChartConfig(items: PieChartItem[]): ChartConfig {
  const config: ChartConfig = {
    value: { label: "Sessions" },
  };
  items.forEach((item, index) => {
    config[item.id] = {
      label: item.label,
      color: chartColor(index),
    };
  });
  return config;
}

function truncateLabel(label: string, max = 14) {
  return label.length > max ? `${label.slice(0, max)}…` : label;
}

export function ProviderPieChart({
  className,
  emptyLabel = "No data",
  items,
}: {
  className?: string;
  emptyLabel?: string;
  items: PieChartItem[];
}) {
  const total = items.reduce((sum, item) => sum + item.value, 0);
  const chartConfig = useMemo(() => buildPieChartConfig(items), [items]);
  const chartData = useMemo(
    () =>
      items.map((item) => ({
        ...item,
        fill: `var(--color-${item.id})`,
      })),
    [items],
  );

  if (!total) {
    return (
      <div className={cn("flex min-h-[12rem] items-center justify-center", className)}>
        <p className="text-sm text-muted-foreground">{emptyLabel}</p>
      </div>
    );
  }

  return (
    <ChartContainer
      config={chartConfig}
      className={cn("mx-auto aspect-square w-full max-h-[220px]", className)}
      initialDimension={{ width: 220, height: 220 }}
    >
      <PieChart>
        <ChartTooltip content={<ChartTooltipContent nameKey="label" />} />
        <Pie
          data={chartData}
          dataKey="value"
          nameKey="label"
          innerRadius={52}
          outerRadius={78}
          paddingAngle={2}
          strokeWidth={2}
          stroke="var(--background)"
        >
          {chartData.map((entry) => (
            <Cell key={entry.id} fill={entry.fill} />
          ))}
        </Pie>
        <ChartLegend content={<ChartLegendContent nameKey="label" />} />
      </PieChart>
    </ChartContainer>
  );
}

export function StatsRankBarChart({
  className,
  emptyLabel = "No data",
  isLoading,
  items,
}: {
  className?: string;
  emptyLabel?: string;
  isLoading?: boolean;
  items: BarChartItem[];
}) {
  const chartData = useMemo(
    () =>
      items.map((item) => ({
        id: item.id,
        label: item.label,
        value: item.value,
      })),
    [items],
  );

  if (isLoading) {
    return (
      <div className={cn("flex min-h-[12rem] flex-col gap-3", className)}>
        <div className="flex flex-1 flex-col justify-center gap-2.5">
          {Array.from({ length: 5 }).map((_, index) => (
            <Skeleton key={index} className="h-5 w-full" style={{ maxWidth: `${88 - index * 10}%` }} />
          ))}
        </div>
      </div>
    );
  }

  if (!items.length) {
    return (
      <div className={cn("flex min-h-[12rem] items-center justify-center", className)}>
        <p className="text-sm text-muted-foreground">{emptyLabel}</p>
      </div>
    );
  }

  return (
    <ChartContainer
      config={rankBarConfig}
      className={cn("min-h-[12rem] w-full", className)}
      initialDimension={{ width: 320, height: 192 }}
    >
      <BarChart
        accessibilityLayer
        data={chartData}
        layout="vertical"
        margin={{ left: 0, right: 8, top: 4, bottom: 4 }}
      >
        <YAxis
          dataKey="label"
          type="category"
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          width={96}
          tick={{ fontSize: 11 }}
          tickFormatter={(value) => truncateLabel(String(value))}
        />
        <XAxis dataKey="value" type="number" hide />
        <ChartTooltip cursor={false} content={<ChartTooltipContent hideLabel indicator="line" nameKey="label" />} />
        <Bar dataKey="value" radius={[0, 4, 4, 0]} maxBarSize={28}>
          {chartData.map((entry, index) => (
            <Cell key={entry.id} fill={chartColor(index)} />
          ))}
        </Bar>
      </BarChart>
    </ChartContainer>
  );
}
