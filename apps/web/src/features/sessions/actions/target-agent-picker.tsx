import { useMemo } from "react";
import { ProviderLogo } from "@/components/shared/provider-logo";
import {
  providerListInstallStatus,
  ProviderListStatusTrailing,
  type ProviderListInstallStatus,
} from "@/components/shared/provider-list-status";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { useAgentsSummary } from "@/features/agents/queries";
import type { ProviderInfo } from "@/lib/types";

export function TargetAgentPicker({
  providers,
  value,
  onChange,
}: {
  providers: ProviderInfo[];
  value: string;
  onChange: (providerId: string) => void;
}) {
  const agentsSummary = useAgentsSummary();

  const installStatusById = useMemo(() => {
    const map = new Map<string, ProviderListInstallStatus>();
    for (const agent of agentsSummary.data?.providers ?? []) {
      map.set(agent.provider_id, providerListInstallStatus(agent, "agent"));
    }
    return map;
  }, [agentsSummary.data?.providers]);

  const ordered = useMemo(
    () =>
      [...providers].sort((left, right) => {
        const leftInstalled = installStatusById.get(left.id) === "installed";
        const rightInstalled = installStatusById.get(right.id) === "installed";
        if (leftInstalled !== rightInstalled) return leftInstalled ? -1 : 1;
        return left.name.localeCompare(right.name);
      }),
    [installStatusById, providers],
  );

  return (
    <ScrollPane
      className="min-h-36 flex-1 rounded-md border"
      innerClassName="flex flex-col gap-2 p-2"
      data-switch-target-agent-list
    >
      {ordered.map((provider) => (
        <SelectableRowButton
          key={provider.id}
          selected={provider.id === value}
          leading={<ProviderLogo providerId={provider.id} size="sm" alt={provider.name} />}
          title={provider.name}
          trailing={
            <ProviderListStatusTrailing status={installStatusById.get(provider.id) ?? "not_installed"} />
          }
          onClick={() => onChange(provider.id)}
        />
      ))}
    </ScrollPane>
  );
}
