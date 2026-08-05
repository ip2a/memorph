import type { AgentManagementEntry } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { useI18n } from "@/lib/i18n-context";

export type ProviderListInstallStatus = "installed" | "not_installed" | "unsupported";

export function providerListInstallStatus(provider: AgentManagementEntry, kind: "agent" | "hook" = "agent"): ProviderListInstallStatus {
  if (kind === "hook") {
    const status = provider.capabilities.hook_management?.status || "";
    if (!status) return "unsupported";
    if (status === "installed_ok" || status.startsWith("installed_")) return "installed";
    if (status === "not_installed") return "not_installed";
    return "unsupported";
  }
  return provider.environment.installed ? "installed" : "not_installed";
}

export function ProviderListInstallStatusBadge({ status }: { status: ProviderListInstallStatus }) {
  const { t } = useI18n();
  const label = status === "installed" ? t("installedStatus") : status === "unsupported" ? t("unsupportedStatus") : t("notInstalled");
  return <Badge variant={status === "installed" ? "secondary" : "outline"}>{label}</Badge>;
}

export function ProviderListStatusTrailing({ attention, status }: { attention?: number; status: ProviderListInstallStatus }) {
  return (
    <span className="flex items-center gap-2">
      {attention ? <Badge variant="destructive">{attention}</Badge> : null}
      <ProviderListInstallStatusBadge status={status} />
    </span>
  );
}
