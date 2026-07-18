import { useQuery } from "@tanstack/react-query";
import { useManagerMeta } from "@/features/manager/queries";
import { getStatsDashboard } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { StatsDashboardRange } from "@/lib/types";

export type StatsWorkspaceScope = "workspace" | "all";

export function useStatsDashboard(range: StatsDashboardRange, scope: StatsWorkspaceScope) {
  const meta = useManagerMeta();
  const workspace = meta.data?.selected_workspace ?? null;
  const all = scope === "all";
  const dashboard = useQuery({
    queryKey: queryKeys.statsDashboard(all, workspace, range),
    queryFn: () => getStatsDashboard({ all, workspace, range }),
    enabled: !meta.isLoading && (all || Boolean(workspace)),
  });
  return { dashboard, meta, workspace, all };
}
