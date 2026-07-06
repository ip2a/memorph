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
    case "all":
    default:
      return 168;
  }
}

export function aggregateActivityValues(timelines: ProviderActivityTimeline[]) {
  if (!timelines.length) return [];
  const length = timelines[0]?.buckets.length ?? 0;
  const totals = Array.from({ length }, () => 0);
  for (const timeline of timelines) {
    timeline.buckets.forEach((bucket, index) => {
      totals[index] = (totals[index] ?? 0) + bucket.activity_score;
    });
  }
  return totals;
}

export function aggregateActivityTimeline(timelines: ProviderActivityTimeline[]) {
  if (!timelines.length) return null;

  const first = timelines[0];
  const buckets = first.buckets.map((bucket, index) => {
    let activity_score = 0;
    let event_count = 0;
    let message_count = 0;
    for (const timeline of timelines) {
      const entry = timeline.buckets[index];
      if (!entry) continue;
      activity_score += entry.activity_score;
      event_count += entry.event_count;
      message_count += entry.message_count;
    }
    return {
      start: bucket.start,
      end: bucket.end,
      event_count,
      message_count,
      activity_score,
    };
  });

  return {
    provider_id: "aggregate",
    session_id: "all",
    created_at: first.range_start,
    last_active_at: first.range_end,
    bucket_unit: "hour" as const,
    bucket_seconds: first.bucket_seconds,
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
    const scanIds = new Set(providers.data?.filter((provider) => provider.scan).map((provider) => provider.id) ?? []);
    const sessionIds = sessions.data?.map((group) => group.provider_id) ?? [];
    const ids = sessionIds.length ? sessionIds.filter((id) => scanIds.has(id)) : [...scanIds];
    return ids.slice(0, 16);
  }, [providers.data, sessions.data]);

  const activityQueries = useQueries({
    queries: activityProviders.map((providerId) => ({
      queryKey: queryKeys.providerActivity(providerId, workspace, hours, allWorkspaces),
      queryFn: () =>
        getProviderActivity(providerId, {
          all: allWorkspaces,
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

  const loading = meta.isLoading || stats.isLoading || hooks.isLoading || sessions.isLoading;

  const error = meta.error || stats.error || hooks.error || sessions.error;

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
