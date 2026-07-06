import { useMemo, useState } from "react";
import { formatChartAxisDateTime } from "@/lib/format";
import type { SessionActivityBucket, SessionActivityTimeline } from "@/lib/types";
import { cn } from "@/lib/utils";

type SessionActivityChartProps = {
  className?: string;
  isLoading?: boolean;
  timeline?: SessionActivityTimeline | null;
};

type ChartPoint = {
  activityScore: number;
  eventCount: number;
  label: string;
  messageCount: number;
  timeLabel: string;
  x: number;
  y: number;
};

const CHART_WIDTH = 320;
const CHART_HEIGHT = 88;
const CHART_INSET = 3;
const CHART_PADDING = { top: 4, right: CHART_INSET, bottom: 14, left: CHART_INSET };

function bucketActivity(bucket: SessionActivityBucket) {
  return bucket.activity_score ?? bucket.event_count + bucket.message_count;
}

function bucketUnitLabel(unit: SessionActivityTimeline["bucket_unit"]) {
  switch (unit) {
    case "minute":
      return "per minute";
    case "hour":
      return "per hour";
    case "twelve_hour":
      return "per 12 hours";
    default:
      return "per bucket";
  }
}

function chartX(index: number, count: number) {
  if (count <= 1) return CHART_WIDTH / 2;
  const innerWidth = CHART_WIDTH - CHART_INSET * 2;
  return CHART_INSET + (index / Math.max(count - 1, 1)) * innerWidth;
}

function buildChartPoints(timeline: SessionActivityTimeline): ChartPoint[] {
  const values = timeline.buckets.map(bucketActivity);
  const max = Math.max(...values, 1);
  const innerHeight = CHART_HEIGHT - CHART_PADDING.top - CHART_PADDING.bottom;
  const count = values.length;

  return timeline.buckets.map((bucket, index) => {
    const activityScore = bucketActivity(bucket);
    return {
      x: chartX(index, count),
      y: CHART_PADDING.top + innerHeight * (1 - activityScore / max),
      activityScore,
      eventCount: bucket.event_count,
      messageCount: bucket.message_count,
      timeLabel: formatChartAxisDateTime(bucket.start, timeline.created_at, timeline.last_active_at),
      label: `${formatChartAxisDateTime(bucket.start, timeline.created_at, timeline.last_active_at)} – ${formatChartAxisDateTime(bucket.end, timeline.created_at, timeline.last_active_at)}`,
    };
  });
}

function buildLinePath(points: ChartPoint[]) {
  if (points.length === 0) return "";
  if (points.length === 1) {
    const point = points[0];
    return `M ${CHART_INSET} ${point.y} L ${CHART_WIDTH - CHART_INSET} ${point.y}`;
  }
  return points.map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`).join(" ");
}

function buildAreaPath(points: ChartPoint[]) {
  if (points.length === 0) return "";
  const baseline = CHART_HEIGHT - CHART_PADDING.bottom;
  const line = buildLinePath(points);
  if (points.length === 1) {
    return `${line} L ${CHART_WIDTH - CHART_INSET} ${baseline} L ${CHART_INSET} ${baseline} Z`;
  }
  const last = points[points.length - 1];
  const first = points[0];
  return `${line} L ${last.x} ${baseline} L ${first.x} ${baseline} Z`;
}

function formatActivityScore(value: number) {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

export function SessionActivityChart({ className, isLoading, timeline }: SessionActivityChartProps) {
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const points = useMemo(() => (timeline ? buildChartPoints(timeline) : []), [timeline]);
  const activePoint = activeIndex === null ? null : points[activeIndex] ?? null;
  const peak = useMemo(
    () => Math.max(...(timeline?.buckets.map(bucketActivity) ?? [0]), 0),
    [timeline],
  );

  if (isLoading) {
    return (
      <div className={cn("flex min-h-[104px] flex-col justify-end gap-2", className)}>
        <div className="h-3 w-40 animate-pulse rounded bg-muted" />
        <div className="h-[88px] animate-pulse rounded bg-muted/70" />
      </div>
    );
  }

  if (!timeline || timeline.buckets.length === 0) {
    return (
      <div className={cn("flex min-h-[104px] flex-col justify-center text-sm text-muted-foreground", className)}>
        No activity timeline
      </div>
    );
  }

  const rangeStartRaw = timeline.created_at ?? timeline.buckets[0]?.start;
  const rangeEndRaw = timeline.last_active_at ?? timeline.buckets[timeline.buckets.length - 1]?.end;
  const rangeStart = formatChartAxisDateTime(rangeStartRaw, rangeStartRaw, rangeEndRaw);
  const rangeEnd = formatChartAxisDateTime(rangeEndRaw, rangeStartRaw, rangeEndRaw);
  const totalActivity = timeline.total_activity ?? timeline.buckets.reduce((sum, bucket) => sum + bucketActivity(bucket), 0);

  return (
    <div className={cn("flex min-h-[104px] min-w-0 flex-col gap-1 overflow-visible", className)} data-session-activity-chart>
      <div className="flex items-start justify-between gap-3 overflow-visible pr-0.5">
        <div className="min-w-0">
          <p className="text-xs font-medium text-foreground">Activity</p>
          <p className="truncate text-[11px] text-muted-foreground">
            {bucketUnitLabel(timeline.bucket_unit)} · peak {formatActivityScore(peak)}
          </p>
        </div>
        {activePoint ? (
          <div className="shrink-0 text-right text-[11px] leading-4 text-muted-foreground">
            <div className="font-medium text-foreground tabular-nums">{activePoint.timeLabel}</div>
            <div>{formatActivityScore(activePoint.activityScore)} activity</div>
            <div>{activePoint.messageCount} msgs · {activePoint.eventCount} events</div>
          </div>
        ) : (
          <div className="shrink-0 whitespace-nowrap text-right text-[11px] tabular-nums text-muted-foreground">
            {formatActivityScore(totalActivity)} total
          </div>
        )}
      </div>

      <div className="relative min-w-0 overflow-visible">
        <svg
          viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
          preserveAspectRatio="none"
          className="block h-[88px] w-full overflow-visible"
          role="img"
          aria-label="Session activity timeline"
          onMouseLeave={() => setActiveIndex(null)}
        >
          <defs>
            <linearGradient id="session-activity-fill" x1="0" x2="0" y1="0" y2="1">
              <stop offset="0%" stopColor="currentColor" stopOpacity="0.28" />
              <stop offset="100%" stopColor="currentColor" stopOpacity="0.02" />
            </linearGradient>
          </defs>
          <line
            x1={CHART_INSET}
            x2={CHART_WIDTH - CHART_INSET}
            y1={CHART_HEIGHT - CHART_PADDING.bottom}
            y2={CHART_HEIGHT - CHART_PADDING.bottom}
            className="stroke-border/70"
            strokeWidth="1"
            vectorEffect="non-scaling-stroke"
          />
          <path d={buildAreaPath(points)} className="fill-[url(#session-activity-fill)] text-primary" vectorEffect="non-scaling-stroke" />
          <path
            d={buildLinePath(points)}
            className="fill-none stroke-primary"
            strokeWidth="2"
            strokeLinejoin="round"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
          {points.map((point, index) => (
            <rect
              key={`${point.label}-${index}`}
              x={index === 0 ? CHART_INSET : (points[index - 1].x + point.x) / 2}
              y={CHART_PADDING.top}
              width={
                index === 0
                  ? (point.x + (points[index + 1]?.x ?? point.x)) / 2 - CHART_INSET
                  : index === points.length - 1
                    ? CHART_WIDTH - CHART_INSET - (points[index - 1].x + point.x) / 2
                    : (point.x - points[index - 1].x) / 2 + ((points[index + 1]?.x ?? point.x) - point.x) / 2
              }
              height={CHART_HEIGHT - CHART_PADDING.top - CHART_PADDING.bottom}
              fill="transparent"
              onMouseEnter={() => setActiveIndex(index)}
            />
          ))}
        </svg>
        {activePoint ? (
          <div
            className="pointer-events-none absolute inset-0"
            aria-hidden
          >
            <div
              className="absolute top-0 bottom-[14px] w-px bg-primary/20"
              style={{ left: `${(activePoint.x / CHART_WIDTH) * 100}%` }}
            />
            <div
              className="absolute size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-background bg-primary shadow-sm ring-1 ring-primary/30"
              style={{
                left: `${(activePoint.x / CHART_WIDTH) * 100}%`,
                top: `${(activePoint.y / CHART_HEIGHT) * 100}%`,
              }}
            />
          </div>
        ) : null}
      </div>

      <div className="grid grid-cols-2 gap-3 text-[11px] tabular-nums text-muted-foreground">
        <span className="min-w-0 truncate">{rangeStart}</span>
        <span className="min-w-0 truncate text-right">{rangeEnd}</span>
      </div>
    </div>
  );
}
