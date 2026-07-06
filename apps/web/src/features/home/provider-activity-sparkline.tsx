import { useQuery } from "@tanstack/react-query";
import { ActivitySparkline } from "@/components/shared/activity-sparkline";
import { getProviderActivity } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function ProviderActivitySparkline({
  providerId,
  workspace,
}: {
  providerId: string;
  workspace?: string | null;
}) {
  const activity = useQuery({
    queryKey: queryKeys.providerActivity(providerId, workspace ?? null),
    queryFn: () => getProviderActivity(providerId, { workspace: workspace ?? undefined, hours: 72 }),
    staleTime: 60_000,
  });

  const values = activity.data?.buckets.map((bucket) => bucket.activity_score) ?? [];
  const title = activity.data
    ? `${Math.round(activity.data.total_activity)} activity over ${activity.data.hours}h`
    : undefined;

  return (
    <ActivitySparkline
      values={values}
      isLoading={activity.isLoading}
      title={title}
      className="h-9 w-[min(32rem,62vw)] shrink-0"
    />
  );
}
