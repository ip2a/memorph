import { describe, expect, it } from "vitest";
import { aggregateActivityTimeline, aggregateActivityValues, statsRangeHours } from "./queries";
import { buildActivityCoverageBlocks, buildActivityLineData } from "./stats-activity-model";
import type { ProviderActivityTimeline, SessionActivityTimeline } from "@/lib/types";

function providerTimeline(
  providerId: string,
  rangeStart: string,
  bucketSeconds: number,
  activity: number[],
): ProviderActivityTimeline {
  const startMs = Date.parse(rangeStart);
  const buckets = activity.map((activity_score, index) => ({
    start: new Date(startMs + index * bucketSeconds * 1000).toISOString(),
    end: new Date(startMs + (index + 1) * bucketSeconds * 1000).toISOString(),
    event_count: activity_score > 0 ? 1 : 0,
    message_count: activity_score > 0 ? 1 : 0,
    activity_score,
  }));
  return {
    provider_id: providerId,
    hours: Math.max(1, Math.ceil((activity.length * bucketSeconds) / 3600)),
    bucket_seconds: bucketSeconds,
    range_start: rangeStart,
    range_end: buckets.at(-1)?.end ?? rangeStart,
    buckets,
    total_activity: activity.reduce((sum, value) => sum + value, 0),
    projected_sessions: 1,
    sessions_with_activity: 1,
  };
}

describe("stats range and activity aggregation", () => {
  it("keeps 30 days distinct from an unbounded all-time request", () => {
    expect(statsRangeHours("24h")).toBe(24);
    expect(statsRangeHours("7d")).toBe(168);
    expect(statsRangeHours("30d")).toBe(720);
    expect(statsRangeHours("all")).toBeUndefined();
  });

  it("resamples provider timelines without losing activity", () => {
    const first = providerTimeline("claude", "2026-07-01T00:00:00.000Z", 3600, [3, 0, 2]);
    const second = providerTimeline("codex", "2026-07-01T01:00:00.000Z", 7200, [4, 1]);

    const aggregate = aggregateActivityTimeline([first, second]);

    expect(aggregate).not.toBeNull();
    expect(aggregate?.total_activity).toBe(10);
    expect(aggregate?.total_events).toBe(4);
    expect(aggregate?.total_messages).toBe(4);
    expect(aggregate?.bucket_seconds).toBe(7200);
    expect(aggregateActivityValues([first, second]).reduce((sum, value) => sum + value, 0)).toBe(10);
  });

  it("ignores malformed provider timelines and malformed buckets", () => {
    const valid = providerTimeline("claude", "2026-07-01T00:00:00.000Z", 3600, [3, 2]);
    valid.buckets.push({
      start: "not-a-date",
      end: "not-a-date",
      event_count: 99,
      message_count: 99,
      activity_score: 99,
    });
    const malformed = providerTimeline("codex", "2026-07-01T00:00:00.000Z", 3600, [4]);
    malformed.range_start = "not-a-date";

    expect(aggregateActivityTimeline([malformed])).toBeNull();
    expect(aggregateActivityTimeline([valid, malformed])?.total_activity).toBe(5);
  });
});

describe("stats performance model", () => {
  it("groups long activity histories without creating empty trailing blocks", () => {
    const source = providerTimeline(
      "claude",
      "2026-07-01T00:00:00.000Z",
      3600,
      Array.from({ length: 31 }, (_, index) => (index === 30 ? 1 : 0)),
    );
    const timeline: SessionActivityTimeline = {
      provider_id: source.provider_id,
      session_id: "all",
      created_at: source.range_start,
      last_active_at: source.range_end,
      bucket_unit: "hour",
      bucket_seconds: source.bucket_seconds,
      buckets: source.buckets,
      total_events: 1,
      total_messages: 1,
      total_activity: 1,
    };

    const blocks = buildActivityCoverageBlocks(timeline, 30);

    expect(blocks).toHaveLength(30);
    expect(blocks.at(-1)?.active).toBe(true);
    expect(buildActivityCoverageBlocks(timeline, 0)).toEqual([]);
    expect(buildActivityLineData(timeline)).toHaveLength(31);
  });
});
