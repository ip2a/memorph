import type { AgentManagementEntry } from "@/lib/types";
import { Badge } from "@/components/ui/badge";

export type ProviderListInstallStatus = "installed" | "not_installed" | "unsupported";

export function providerListInstallStatus(
  provider: AgentManagementEntry,
  kind: "hook" | "agent",
): ProviderListInstallStatus {
  if (!provider.hook_profile) return "unsupported";
  if (kind === "agent") {
    const installed = provider.environment?.installed ?? !!provider.installed;
    return installed ? "installed" : "not_installed";
  }
  const status = provider.hook?.status || "";
  if (status === "installed_ok" || (status.startsWith("installed_") && status !== "not_installed")) {
    return "installed";
  }
  if (status === "not_installed") return "not_installed";
  return "unsupported";
}

function statusLabel(status: ProviderListInstallStatus): string {
  if (status === "installed") return "Installed";
  return status;
}

export function ProviderListInstallStatusBadge({ status }: { status: ProviderListInstallStatus }) {
  return (
    <Badge variant={status === "installed" ? "secondary" : "outline"}>
      {statusLabel(status)}
    </Badge>
  );
}

export function providerHookAttention(provider: AgentManagementEntry): number {
  const diagnosis = provider.hook_diagnosis || {};
  return (
    Number(diagnosis.hook_needs_attention || 0) +
    Number(diagnosis.no_session_match || 0) +
    Number(diagnosis.no_active_runtime || 0) +
    Number(diagnosis.no_events_yet || 0) +
    Number(diagnosis.hook_not_installed || 0)
  );
}

export function ProviderListStatusTrailing({
  attention,
  status,
}: {
  attention?: number;
  status: ProviderListInstallStatus;
}) {
  return (
    <span className="flex items-center gap-2">
      {attention ? <Badge variant="destructive">{attention}</Badge> : null}
      <ProviderListInstallStatusBadge status={status} />
    </span>
  );
}
