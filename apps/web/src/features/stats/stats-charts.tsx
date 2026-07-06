import { useMemo, useState, type ReactNode } from "react";
import { ActivitySparkline } from "@/components/shared/activity-sparkline";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export type BarChartItem = {
  id: string;
  label: string;
  value: number;
};

export type PieChartItem = {
  colorClassName?: string;
  id: string;
  label: string;
  value: number;
};

export type SegmentItem = {
  className: string;
  label: string;
  value: number;
};

const PIE_SLICE_FILLS = [
  "fill-primary",
  "fill-primary/80",
  "fill-primary/65",
  "fill-primary/50",
  "fill-primary/35",
  "fill-muted-foreground/45",
] as const;

function polarToCartesian(cx: number, cy: number, radius: number, angleDeg: number) {
  const radians = (angleDeg * Math.PI) / 180 - Math.PI / 2;
  return {
    x: cx + radius * Math.cos(radians),
    y: cy + radius * Math.sin(radians),
  };
}

function buildPieSlicePath(cx: number, cy: number, radius: number, startAngle: number, endAngle: number) {
  if (endAngle - startAngle >= 359.999) {
    const top = polarToCartesian(cx, cy, radius, 0);
    const bottom = polarToCartesian(cx, cy, radius, 180);
    return `M ${cx} ${cy} L ${top.x} ${top.y} A ${radius} ${radius} 0 1 1 ${bottom.x} ${bottom.y} A ${radius} ${radius} 0 1 1 ${top.x} ${top.y} Z`;
  }

  const start = polarToCartesian(cx, cy, radius, startAngle);
  const end = polarToCartesian(cx, cy, radius, endAngle);
  const largeArc = endAngle - startAngle > 180 ? 1 : 0;
  return `M ${cx} ${cy} L ${start.x} ${start.y} A ${radius} ${radius} 0 ${largeArc} 1 ${end.x} ${end.y} Z`;
}

function buildPieSlices(items: PieChartItem[], total: number) {
  let cursor = 0;
  return items.map((item, index) => {
    const sweep = (item.value / total) * 360;
    const startAngle = cursor;
    const endAngle = cursor + sweep;
    cursor = endAngle;
    return {
      ...item,
      d: buildPieSlicePath(50, 50, 42, startAngle, endAngle),
      fillClassName: item.colorClassName ?? PIE_SLICE_FILLS[Math.min(index, PIE_SLICE_FILLS.length - 1)],
      percent: (item.value / total) * 100,
    };
  });
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
  const slices = useMemo(() => (total ? buildPieSlices(items, total) : []), [items, total]);
  const [activeId, setActiveId] = useState<string | null>(null);

  if (!total) {
    return <p className="text-muted-foreground text-sm">{emptyLabel}</p>;
  }

  return (
    <div className={cn("flex min-w-0 flex-col items-center justify-center gap-3", className)}>
      <svg viewBox="0 0 100 100" className="aspect-square w-full max-w-36 shrink-0" role="img" aria-label="Provider share pie chart">
        {slices.map((slice) => {
          const active = activeId === slice.id;
          return (
            <Tooltip key={slice.id}>
              <TooltipTrigger asChild>
                <g
                  className="cursor-pointer outline-none"
                  onMouseEnter={() => setActiveId(slice.id)}
                  onMouseLeave={() => setActiveId(null)}
                  onFocus={() => setActiveId(slice.id)}
                  onBlur={() => setActiveId(null)}
                >
                  <path
                    d={slice.d}
                    className={cn(
                      slice.fillClassName,
                      "stroke-background transition-opacity",
                      active ? "opacity-100" : activeId ? "opacity-35" : "opacity-90 hover:opacity-100",
                    )}
                    strokeWidth={0.6}
                  />
                </g>
              </TooltipTrigger>
              <TooltipContent sideOffset={6}>
                <span className="font-medium">{slice.label}</span>
                <span className="opacity-80">
                  {" "}
                  · {slice.value} ({slice.percent.toFixed(1)}%)
                </span>
              </TooltipContent>
            </Tooltip>
          );
        })}
      </svg>
    </div>
  );
}

export function StatsRankBarChart({
  className,
  emptyLabel = "No data",
  isLoading,
  items,
  title,
}: {
  className?: string;
  emptyLabel?: string;
  isLoading?: boolean;
  items: BarChartItem[];
  title?: string;
}) {
  if (isLoading) {
    return (
      <div className={cn("flex min-h-[5rem] min-w-0 flex-col gap-2", className)}>
        {title ? <div className="h-3 w-24 animate-pulse rounded bg-muted" /> : null}
        <div className="flex min-h-0 flex-1 items-end gap-1.5">
          {Array.from({ length: 5 }).map((_, index) => (
            <div key={index} className="flex-1 animate-pulse rounded-t bg-muted/70" style={{ height: `${40 + index * 8}%` }} />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className={cn("flex min-h-[5rem] min-w-0 flex-col gap-2", className)}>
      {title ? <p className="truncate text-[11px] font-medium text-muted-foreground">{title}</p> : null}
      <VerticalBarChart emptyLabel={emptyLabel} items={items} />
    </div>
  );
}

export function VerticalBarChart({
  className,
  emptyLabel = "No data",
  items,
}: {
  className?: string;
  emptyLabel?: string;
  items: BarChartItem[];
}) {
  const max = Math.max(...items.map((item) => item.value), 1);
  const [activeId, setActiveId] = useState<string | null>(null);

  if (!items.length) {
    return <p className="text-muted-foreground text-sm">{emptyLabel}</p>;
  }

  return (
    <div className={cn("flex min-h-[4.75rem] flex-1 items-end gap-1 sm:gap-1.5", className)}>
      {items.map((item) => {
        const active = activeId === item.id;
        const height = Math.max((item.value / max) * 100, item.value > 0 ? 8 : 0);
        return (
          <Tooltip key={item.id}>
            <TooltipTrigger asChild>
              <div
                className="flex min-w-0 flex-1 cursor-pointer flex-col items-center gap-1 outline-none"
                onMouseEnter={() => setActiveId(item.id)}
                onMouseLeave={() => setActiveId(null)}
                onFocus={() => setActiveId(item.id)}
                onBlur={() => setActiveId(null)}
              >
                <div className="flex h-20 w-full items-end justify-center sm:h-24">
                  <div
                    className={cn(
                      "w-full max-w-6 rounded-t-sm bg-primary transition-[height,opacity]",
                      active ? "opacity-100" : activeId ? "opacity-35" : "opacity-90 hover:opacity-100",
                    )}
                    style={{ height: `${height}%` }}
                  />
                </div>
                <span className="w-full truncate text-center font-mono text-[9px] text-muted-foreground" title={item.label}>
                  {item.label}
                </span>
              </div>
            </TooltipTrigger>
            <TooltipContent sideOffset={6}>
              <span className="font-medium">{item.label}</span>
              <span className="opacity-80"> · {item.value}</span>
            </TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}

export function HorizontalBarChart({
  className,
  compact = false,
  emptyLabel = "No data",
  items,
}: {
  className?: string;
  compact?: boolean;
  emptyLabel?: string;
  items: BarChartItem[];
}) {
  const max = Math.max(...items.map((item) => item.value), 1);

  if (!items.length) {
    return <p className="text-muted-foreground text-sm">{emptyLabel}</p>;
  }

  return (
    <div className={cn("flex flex-col", compact ? "gap-1.5" : "gap-2.5", className)}>
      {items.map((item) => (
        <div
          key={item.id}
          className={cn(
            "grid items-center gap-1.5",
            compact
              ? "grid-cols-[minmax(0,3.75rem)_minmax(0,1fr)_1.75rem]"
              : "grid-cols-[minmax(0,5.5rem)_minmax(0,1fr)_2.25rem] gap-2",
          )}
        >
          <span className="truncate font-mono text-[10px] text-muted-foreground" title={item.label}>
            {item.label}
          </span>
          <div className={cn("overflow-hidden rounded-full bg-muted/70", compact ? "h-1.5" : "h-2")}>
            <div
              className="h-full rounded-full bg-primary transition-[width]"
              style={{ width: `${(item.value / max) * 100}%` }}
            />
          </div>
          <span className="text-right font-mono text-[10px] tabular-nums">{item.value}</span>
        </div>
      ))}
    </div>
  );
}

export function SegmentBarChart({
  className,
  emptyLabel = "No data",
  items,
}: {
  className?: string;
  emptyLabel?: string;
  items: SegmentItem[];
}) {
  const total = items.reduce((sum, item) => sum + item.value, 0);

  if (!total) {
    return <p className="text-muted-foreground text-sm">{emptyLabel}</p>;
  }

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      <div className="flex h-3 overflow-hidden rounded-full bg-muted/50">
        {items
          .filter((item) => item.value > 0)
          .map((item) => (
            <div
              key={item.label}
              className={cn("h-full min-w-0 transition-[width]", item.className)}
              style={{ width: `${(item.value / total) * 100}%` }}
              title={`${item.label}: ${item.value}`}
            />
          ))}
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {items.map((item) => (
          <div key={item.label} className="flex items-center gap-2 text-xs">
            <span className={cn("size-2 shrink-0 rounded-full", item.className.replace("/80", ""))} />
            <span className="text-muted-foreground">{item.label}</span>
            <span className="ml-auto font-mono tabular-nums">{item.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function StatsActivityChart({
  className,
  isLoading,
  subtitle,
  title,
  values,
}: {
  className?: string;
  isLoading?: boolean;
  subtitle?: ReactNode;
  title?: string;
  values: number[];
}) {
  return (
    <div className={cn("flex min-h-44 flex-col gap-3", className)}>
      {subtitle ? <p className="text-muted-foreground text-xs">{subtitle}</p> : null}
      <ActivitySparkline
        values={values}
        isLoading={isLoading}
        title={title}
        height={176}
        className="h-44 w-full rounded-lg"
      />
    </div>
  );
}

export function StatsMetricList({
  items,
}: {
  items: Array<{ label: string; value: ReactNode; hint?: ReactNode }>;
}) {
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {items.map((item) => (
        <div key={item.label} className="rounded-md border bg-muted/20 px-3 py-2">
          <div className="text-muted-foreground font-mono text-xs uppercase">{item.label}</div>
          <div className="mt-1 font-semibold">{item.value}</div>
          {item.hint ? <div className="text-muted-foreground mt-1 text-xs">{item.hint}</div> : null}
        </div>
      ))}
    </div>
  );
}
