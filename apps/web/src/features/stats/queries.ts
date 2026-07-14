import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import { getProviderActivity, listSessions } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ProviderActivityTimeline, SessionGroup } from "@/lib/types";
import { sessionTitle } from "@/lib/format";
import { workspaceName } from "@/components/shared/workspace-name";
import { useHooksOverview } from "@/features/hooks/queries";
import { useManagerMeta, useManagerProviders, useManagerStats } from "@/features/manager/queries";

export type StatsRange = "24h" | "7d" | "30d" | "all";
export type StatsWorkspaceScope = "workspace" | "all";

const MANAGER_FILTER = { sort: "recent" as const, limit: 500 };

export function statsRangeHours(range: StatsRange) {
  switch (range) {
    case "24h":
      return 24;
    case "7d":
      return 168;
    case "30d":
      return 720;
    case "all":
    default:
      return undefined;
  }
}

export function aggregateActivityValues(timelines: ProviderActivityTimeline[]) {
  return aggregateActivityTimeline(timelines)?.buckets.map((bucket) => bucket.activity_score) ?? [];
}

export function aggregateActivityTimeline(timelines: ProviderActivityTimeline[]) {
  const validTimelines = timelines.filter((timeline) => {
    const start = Date.parse(timeline.range_start);
    const end = Date.parse(timeline.range_end);
    return Number.isFinite(start) && Number.isFinite(end) && end >= start && timeline.bucket_seconds > 0;
  });
  if (!validTimelines.length) return null;

  const rangeStartMs = Math.min(...validTimelines.map((timeline) => Date.parse(timeline.range_start)));
  const rangeEndMs = Math.max(...validTimelines.map((timeline) => Date.parse(timeline.range_end)));
  let bucketSeconds = Math.max(...validTimelines.map((timeline) => timeline.bucket_seconds), 1);
  const spanSeconds = Math.max(0, Math.ceil((rangeEndMs - rangeStartMs) / 1000));
  while (Math.ceil(spanSeconds / bucketSeconds) > 120) {
    bucketSeconds *= 2;
  }
  const bucketCount = Math.max(1, Math.ceil(spanSeconds / bucketSeconds));
  const buckets = Array.from({ length: bucketCount }, (_, index) => {
    const startMs = rangeStartMs + index * bucketSeconds * 1000;
    const endMs = index + 1 === bucketCount ? rangeEndMs : startMs + bucketSeconds * 1000;
    return {
      start: new Date(startMs).toISOString(),
      end: new Date(endMs).toISOString(),
      event_count: 0,
      message_count: 0,
      activity_score: 0,
    };
  });

  for (const timeline of validTimelines) {
    for (const source of timeline.buckets) {
      const sourceStartMs = Date.parse(source.start);
      if (!Number.isFinite(sourceStartMs)) continue;
      const index = Math.min(
        Math.max(Math.floor((sourceStartMs - rangeStartMs) / (bucketSeconds * 1000)), 0),
        buckets.length - 1,
      );
      const target = buckets[index];
      target.event_count += source.event_count;
      target.message_count += source.message_count;
      target.activity_score += source.activity_score;
    }
  }

  return {
    provider_id: "aggregate",
    session_id: "all",
    created_at: new Date(rangeStartMs).toISOString(),
    last_active_at: new Date(rangeEndMs).toISOString(),
    bucket_unit:
      bucketSeconds === 60
        ? ("minute" as const)
        : bucketSeconds === 3600
          ? ("hour" as const)
          : bucketSeconds === 43_200
            ? ("twelve_hour" as const)
            : ("adaptive" as const),
    bucket_seconds: bucketSeconds,
    buckets,
    total_events: buckets.reduce((sum, bucket) => sum + bucket.event_count, 0),
    total_messages: buckets.reduce((sum, bucket) => sum + bucket.message_count, 0),
    total_activity: buckets.reduce((sum, bucket) => sum + bucket.activity_score, 0),
  };
}

export function providerSessionBreakdown(groups: SessionGroup[] | undefined) {
  return (groups ?? [])
    .map((group) => ({
      id: group.provider_id,
      label: group.provider_name || group.provider_id,
      value: group.sessions.length,
    }))
    .filter((item) => item.value > 0)
    .sort((left, right) => right.value - left.value);
}

export function topSessionsByMessages(groups: SessionGroup[] | undefined, limit = 5) {
  return (groups ?? [])
    .flatMap((group) =>
      group.sessions.map((session) => ({
        id: `${group.provider_id}:${session.session_id}`,
        label: sessionTitle(session),
        value: session.message_count ?? 0,
      })),
    )
    .filter((item) => item.value > 0)
    .sort((left, right) => right.value - left.value)
    .slice(0, limit);
}

export function topWorkspacesByMessages(groups: SessionGroup[] | undefined, limit = 5) {
  const totals = new Map<string, { label: string; value: number }>();

  for (const group of groups ?? []) {
    for (const session of group.sessions) {
      const workspace = session.project_dir || "unknown";
      const current = totals.get(workspace) ?? { label: workspaceName(workspace), value: 0 };
      current.value += session.message_count ?? 0;
      totals.set(workspace, current);
    }
  }

  return [...totals.entries()]
    .map(([id, item]) => ({ id, label: item.label, value: item.value }))
    .filter((item) => item.value > 0)
    .sort((left, right) => right.value - left.value)
    .slice(0, limit);
}

export function useStatsDashboard(range: StatsRange, scope: StatsWorkspaceScope) {
  const hours = statsRangeHours(range);
  const allWorkspaces = scope === "all";
  const meta = useManagerMeta();
  const workspace = meta.data?.selected_workspace ?? null;
  const stats = useManagerStats(MANAGER_FILTER);
  const hooks = useHooksOverview();
  const providers = useManagerProviders();

  const sessions = useQuery({
    queryKey: queryKeys.sessions({
      all: allWorkspaces,
      workspace: allWorkspaces ? undefined : workspace ?? undefined,
      limit: 500,
      sort: "recent",
      details: true,
    }),
    queryFn: () =>
      listSessions({
        all: allWorkspaces,
        workspace: allWorkspaces ? undefined : workspace ?? undefined,
        limit: 500,
        sort: "recent",
        details: true,
      }),
    enabled: !meta.isLoading,
  });

  const activityProviders = useMemo(() => {
    const ids = new Set(providers.data?.map((provider) => provider.id) ?? []);
    for (const group of sessions.data ?? []) {
      ids.add(group.provider_id);
    }
    return [...ids];
  }, [providers.data, sessions.data]);

  const activityQueries = useQueries({
    queries: activityProviders.map((providerId) => ({
      queryKey: queryKeys.providerActivity(providerId, workspace, hours ?? "all", allWorkspaces),
      queryFn: () =>
        getProviderActivity(providerId, {
          all: allWorkspaces,
          all_time: range === "all",
          hours,
          workspace: allWorkspaces ? undefined : workspace ?? undefined,
        }),
      staleTime: 60_000,
      enabled: !meta.isLoading && activityProviders.length > 0,
    })),
  });

  const activityTimelines = useMemo(
    () => activityQueries.flatMap((query) => (query.data ? [query.data] : [])),
    [activityQueries],
  );

  const activityValues = useMemo(() => aggregateActivityValues(activityTimelines), [activityTimelines]);
  const activityTimeline = useMemo(() => aggregateActivityTimeline(activityTimelines), [activityTimelines]);

  const providerBreakdown = useMemo(() => providerSessionBreakdown(sessions.data), [sessions.data]);
  const rankBarItems = useMemo(
    () => (allWorkspaces ? topWorkspacesByMessages(sessions.data) : topSessionsByMessages(sessions.data)),
    [allWorkspaces, sessions.data],
  );

  const activityLoading = activityQueries.some((query) => query.isLoading);
  const activityTotal = useMemo(() => activityValues.reduce((sum, value) => sum + value, 0), [activityValues]);

  const loading = meta.isLoading || stats.isLoading || hooks.isLoading || providers.isLoading || sessions.isLoading;

  const activityError = activityQueries.find((query) => query.error)?.error;
  const error = meta.error || stats.error || hooks.error || providers.error || sessions.error || activityError;

  return {
    activityLoading,
    activityProviders,
    activityTimeline,
    activityTotal,
    activityValues,
    allWorkspaces,
    hooks,
    hours,
    loading,
    meta,
    providerBreakdown,
    rankBarItems,
    providers,
    sessions,
    stats,
    workspace,
    error,
  };
}
