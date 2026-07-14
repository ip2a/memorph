import { formatChartAxisDateTime } from "@/lib/format";
import type { SessionActivityTimeline } from "@/lib/types";

export type UsageTableItem = {
  id: string;
  label: string;
  value: number;
};

export type ActivityCoverageBlock = {
  id: string;
  active: boolean;
};

function bucketActivity(bucket: SessionActivityTimeline["buckets"][number]) {
  return bucket.activity_score ?? bucket.event_count + bucket.message_count;
}

function formatShortDate(
  value: number | string | null | undefined,
  rangeStart: number | string | null | undefined,
  rangeEnd: number | string | null | undefined,
) {
  const label = formatChartAxisDateTime(value, rangeStart, rangeEnd);
  if (label.includes(" ")) {
    const [datePart] = label.split(" ");
    const [month, day] = datePart.split("-");
    const monthNames = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const monthIndex = Number(month) - 1;
    if (monthIndex >= 0 && monthIndex < 12 && day) {
      return `${monthNames[monthIndex]} ${Number(day)}`;
    }
  }
  return label;
}

export function buildActivityCoverageBlocks(
  timeline: SessionActivityTimeline | null | undefined,
  maxBlocks = 30,
): ActivityCoverageBlock[] {
  if (!timeline?.buckets.length || maxBlocks < 1) return [];

  const buckets = timeline.buckets;
  const blockCount = Math.min(maxBlocks, buckets.length);
  return Array.from({ length: blockCount }, (_, index) => {
    const start = Math.floor((index * buckets.length) / blockCount);
    const end = Math.floor(((index + 1) * buckets.length) / blockCount);
    const slice = buckets.slice(start, Math.max(start + 1, end));
    return {
      id: `group-${index}`,
      active: slice.some((bucket) => bucketActivity(bucket) > 0),
    };
  });
}

export function buildActivityLineData(timeline: SessionActivityTimeline | null | undefined) {
  if (!timeline?.buckets.length) return [];

  const rangeStart = timeline.created_at ?? timeline.buckets[0]?.start;
  const rangeEnd = timeline.last_active_at ?? timeline.buckets[timeline.buckets.length - 1]?.end;

  return timeline.buckets.map((bucket, index) => ({
    id: `${bucket.start}-${index}`,
    label: formatShortDate(bucket.start, rangeStart, rangeEnd),
    activity: bucketActivity(bucket),
  }));
}
