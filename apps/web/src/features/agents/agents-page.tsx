import { useMemo, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { InfoIcon, PlayIcon, RefreshCwIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PanelCard } from "@/components/shared/panel-card";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { SectionHeading } from "@/components/shared/section-heading";
import {
  providerListInstallStatus,
  ProviderListStatusTrailing,
} from "@/components/shared/provider-list-status";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { formatBytes, formatExecutableVersion } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useI18n } from "@/lib/i18n-context";
import type {
  AgentEnvironmentStatus,
  AgentManagementEntry,
  ProviderCapabilities,
  ProviderContentFidelity,
  ProviderSettingItem,
} from "@/lib/types";
import {
  prefetchAgent,
  useAgent,
  useAgentDetectedHooks,
  useAgentProviderCatalog,
  useAgentsMeta,
  useAgentsSummary,
  useDetectAgent,
  useDeleteAgentDetectedHook,
  useRunAgentHookOperation,
  useRunProviderSetting,
  useUpdateProviderSetting,
} from "@/features/agents/queries";
import { useManagerStats } from "@/features/manager/queries";
import { AgentActionResultPanel } from "@/features/agents/agent-action-result";
import { ConfigViewsBlock } from "@/features/agents/config-views-panel";

const HOOK_SETTING_IDS = new Set(["install_hook", "verify_hook", "repair_hook", "uninstall_hook"]);
type DetailTab = "overview" | "hooks" | "mcp" | "plugins" | "config";

function environmentOf(provider: AgentManagementEntry): AgentEnvironmentStatus {
  return provider.environment;
}

function providerSettings(provider: AgentManagementEntry) {
  return (provider.settings || []).filter(
    (setting) => (setting.kind === "toggle" || setting.kind === "action") && !HOOK_SETTING_IDS.has(setting.id),
  );
}

function settingLabel(setting: ProviderSettingItem, t: ReturnType<typeof useI18n>["t"]) {
  if (setting.id === "repair_workspace_sessions") return t("repairWorkspaceSessions");
  if (setting.id === "show_subagents") return t("showSubagents");
  return setting.title || setting.id;
}

function DetailSection({
  title,
  description,
  actions,
  actionsClassName,
  headingClassName,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  actionsClassName?: string;
  headingClassName?: string;
  children?: ReactNode;
}) {
  return (
    <section className={cn("flex flex-col border-t pt-5", children ? "gap-4" : "")}>
      <SectionHeading
        title={title}
        description={description}
        actions={actions}
        className={cn("border-b-0 pb-0", headingClassName)}
        actionsProps={{ className: actionsClassName }}
      />
      {children}
    </section>
  );
}

function DetailRow({ label, value, hint, actions }: { label: string; value: string | number | null | undefined; hint?: string; actions?: ReactNode }) {
  const display = value || "-";
  const title = typeof value === "string" && value ? value : undefined;

  return (
    <div
      className={cn(
        "grid gap-3 border-b py-3",
        actions ? "md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)_auto]" : "md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]",
      )}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <strong className="text-sm font-medium">{label}</strong>
        {hint ? <span className="text-muted-foreground text-xs">{hint}</span> : null}
      </div>
      <div className="text-muted-foreground min-w-0 truncate font-mono text-xs" title={title}>
        {display}
      </div>
      {actions ? <div className="flex items-start justify-end">{actions}</div> : null}
    </div>
  );
}

function SummaryGrid({ items }: { items: Array<{ label: string; value: string | number | null | undefined }> }) {
  return (
    <MetricGrid>
      {items.map((item) => <MetricTile key={item.label} label={item.label} value={item.value || "-"} />)}
    </MetricGrid>
  );
}

function ProviderList({
  providers,
  selectedProvider,
  onSelect,
  onPrefetch,
}: {
  providers: AgentManagementEntry[];
  selectedProvider: string | null;
  onSelect: (provider: string) => void;
  onPrefetch: (provider: string) => void;
}) {
  const { t } = useI18n();
  const ordered = useMemo(
    () =>
      [...providers].sort((left, right) => {
        const leftInstalled = providerListInstallStatus(left, "agent") === "installed";
        const rightInstalled = providerListInstallStatus(right, "agent") === "installed";
        if (leftInstalled !== rightInstalled) return leftInstalled ? -1 : 1;
        return (left.name || left.provider_id).localeCompare(right.name || right.provider_id);
      }),
    [providers],
  );

  if (ordered.length === 0) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>{t("agentsNoProviders")}</EmptyTitle>
          <EmptyDescription>{t("agentsNoProvidersDescription")}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-2">
      {ordered.map((provider) => {
          const selected = provider.provider_id === selectedProvider;
          const status = providerListInstallStatus(provider, "agent");
          return (
            <SelectableRowButton
              key={provider.provider_id}
              selected={selected}
              leading={<ProviderLogo providerId={provider.provider_id} size="sm" alt={provider.name || provider.provider_id} />}
              title={provider.name || provider.provider_id}
              trailing={<ProviderListStatusTrailing status={status} />}
              onClick={() => onSelect(provider.provider_id)}
              onFocus={() => onPrefetch(provider.provider_id)}
              onPointerEnter={() => onPrefetch(provider.provider_id)}
            />
          );
        })}
    </ScrollPane>
  );
}

function EnvironmentBlock({ provider }: { provider: AgentManagementEntry }) {
  const { t } = useI18n();
  const environment = environmentOf(provider);
  return (
    <DetailSection title={t("agentsEnvironment")}>
      <div className="flex flex-col">
        <DetailRow label={t("executablePath")} value={environment.executable_path} />
        <DetailRow label={t("executableDir")} value={environment.executable_dir} />
        <DetailRow label={t("executableVersion")} value={formatExecutableVersion(environment.executable_version)} />
        <DetailRow label={t("configPath")} value={environment.config_path} />
      </div>
    </DetailSection>
  );
}

function AgentStatsStrip({
  provider,
  loading,
}: {
  provider: AgentManagementEntry | undefined;
  loading: boolean;
}) {
  const { t } = useI18n();
  const providerId = provider?.provider_id;
  const filter = useMemo(() => ({ providers: providerId ? [providerId] : [] }), [providerId]);
  const stats = useManagerStats(filter, { enabled: !!providerId });
  const environment = provider ? environmentOf(provider) : null;
  const sessionCount = stats.data?.all_workspace_session_count ?? 0;
  const version = formatExecutableVersion(environment?.executable_version);
  const placeholder = loading || stats.isLoading ? <Skeleton className="h-5 w-20" /> : "-";

  return (
    <MetricGrid columns="four" data-agent-stats>
      <MetricTile
        label={t("sessions")}
        value={provider ? sessionCount : placeholder}
        hint={t("allWorkspaces")}
        variant="compact"
      />
      <MetricTile
        label={t("size")}
        value={stats.data ? formatBytes(stats.data.all_workspace_size_bytes) : placeholder}
        hint={t("indexedStorage")}
        variant="compact"
      />
      <MetricTile
        label={t("version")}
        value={version || placeholder}
        hint={t("executable")}
        variant="compact"
        title={version ?? undefined}
      />
      <MetricTile
        label={t("installMethod")}
        value={environment?.install_method || placeholder}
        variant="compact"
      />
    </MetricGrid>
  );
}

function HooksBlock({ provider }: { provider: AgentManagementEntry }) {
  const { t } = useI18n();
  const capability = provider.capabilities.hook_management;
  const detectedHooks = useAgentDetectedHooks(provider.provider_id, capability?.discovery === true);
  const operation = useRunAgentHookOperation();
  const deleteHook = useDeleteAgentDetectedHook();

  if (!capability) return null;

  const operations = [
    ["install_hook", capability.install, t("install")],
    ["verify_hook", capability.verify, t("verify")],
    ["repair_hook", capability.repair, t("repair")],
    ["uninstall_hook", capability.uninstall, t("uninstall")],
  ] as const;

  return (
    <DetailSection
      title={t("hooks")}
      description={`${t("status")}: ${capability.status}`}
      headingClassName="md:items-center"
      actionsClassName="items-center"
      actions={<Badge variant={capability.status === "installed_ok" ? "secondary" : "outline"}>{capability.status}</Badge>}
    >
      <div className="flex flex-wrap gap-2">
        {operations.map(([id, supported, label]) => supported ? (
          <Button
            key={id}
            type="button"
            variant="outline"
            size="sm"
            disabled={operation.isPending}
            onClick={() => operation.mutate({ provider: provider.provider_id, operation: id })}
          >
            {operation.isPending && operation.variables?.operation === id ? <Spinner data-icon="inline-start" /> : null}
            {label}
          </Button>
        ) : null)}
      </div>
      {detectedHooks.isLoading ? <Skeleton className="h-10 w-full" /> : null}
      {detectedHooks.data?.hooks.length ? (
        <div className="flex flex-col gap-2">
          {detectedHooks.data.hooks.map((hook) => (
            <DetailRow
              key={`${hook.event}:${hook.index}:${hook.fingerprint}`}
              label={`${hook.event} #${hook.index + 1}`}
              value={hook.command || hook.hook_type || hook.fingerprint}
              actions={
                <Button
                  type="button"
                  variant="destructive"
                  size="sm"
                  disabled={operation.isPending || deleteHook.isPending}
                  onClick={() =>
                    deleteHook.mutate({
                      provider: provider.provider_id,
                      event: hook.event,
                      index: hook.index,
                      fingerprint: hook.fingerprint,
                    })
                  }
                >
                  {deleteHook.isPending &&
                  deleteHook.variables?.provider === provider.provider_id &&
                  deleteHook.variables?.event === hook.event &&
                  deleteHook.variables?.index === hook.index &&
                  deleteHook.variables?.fingerprint === hook.fingerprint ? (
                    <Spinner data-icon="inline-start" />
                  ) : null}
                  {t("remove")}
                </Button>
              }
            />
          ))}
        </div>
      ) : null}
      {operation.error ? <PageError title={t("hookOperationFailed")} message={operation.error.message} /> : null}
      {deleteHook.error ? <PageError title={t("hookDeleteFailed")} message={deleteHook.error.message} /> : null}
    </DetailSection>
  );
}

function capabilityLabel(value: string) {
  return value.replaceAll("_", " ");
}

function riskVariant(level: string): "secondary" | "outline" | "destructive" {
  if (level === "high" || level === "unknown") return "destructive";
  return level === "low" ? "secondary" : "outline";
}

function FidelityRows({
  label,
  fidelity,
}: {
  label: string;
  fidelity: ProviderContentFidelity;
}) {
  const { t } = useI18n();
  const entries = Object.entries(fidelity).filter((entry): entry is [string, string] => Boolean(entry[1]));
  return (
    <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
      <strong className="text-sm font-medium">{label}</strong>
      <div className="flex flex-wrap gap-1.5">
        {entries.length ? entries.map(([kind, disposition]) => (
          <Badge
            key={kind}
            variant={disposition === "dropped" || disposition === "unsupported" ? "destructive" : disposition === "preserved" ? "secondary" : "outline"}
          >
            {capabilityLabel(kind)}: {capabilityLabel(disposition)}
          </Badge>
        )) : <span className="text-xs text-muted-foreground">{t("unknown")}</span>}
      </div>
    </div>
  );
}

function CapabilityContent({ capabilities }: { capabilities: ProviderCapabilities | undefined }) {
  const { t } = useI18n();
  if (!capabilities) {
    return (
      <div className="text-sm text-muted-foreground">{t("capabilityUnavailable")}</div>
    );
  }

  const operations = [
    [t("scan"), capabilities.scan],
    [t("import"), capabilities.import],
    [t("export"), capabilities.export],
    [t("remove"), capabilities.delete],
    [t("rename"), capabilities.rename],
    [t("resume"), capabilities.resume],
  ] as const;
  const topology = [
    [t("multipleFiles"), capabilities.write_risk.multiple_files],
    [t("sqlite"), capabilities.write_risk.sqlite],
    [t("sidecars"), capabilities.write_risk.sidecar_files],
    [t("indexRepair"), capabilities.write_risk.index_repair],
  ] as const;

  return (
    <>
      <SummaryGrid
        items={[
          { label: t("storage"), value: capabilityLabel(capabilities.storage_shape) },
          { label: t("scan"), value: capabilityLabel(capabilities.scan_strategy) },
          { label: t("paging"), value: capabilityLabel(capabilities.page_strategy) },
          { label: t("turnQuality"), value: capabilityLabel(capabilities.turn_quality) },
          { label: t("resume"), value: capabilityLabel(capabilities.resume_quality) },
          { label: t("writeRisk"), value: capabilityLabel(capabilities.write_risk.level) },
        ]}
      />
      <div className="flex flex-col">
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">{t("operations")}</strong>
          <div className="flex flex-wrap gap-1.5">
            {operations.map(([label, supported]) => (
              <Badge key={label} variant={supported ? "secondary" : "outline"}>{label}: {supported ? t("yes") : t("no")}</Badge>
            ))}
          </div>
        </div>
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <div className="flex min-w-0 flex-col gap-1">
            <strong className="text-sm font-medium">{t("nativeWriteRisk")}</strong>
            <span className="text-xs text-muted-foreground">{t("nativeWriteRiskDescription")}</span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={riskVariant(capabilities.write_risk.level)}>{capabilityLabel(capabilities.write_risk.level)}</Badge>
            {topology.map(([label, present]) => (
              <Badge key={label} variant={present ? "outline" : "ghost"}>{label}: {present ? t("yes") : t("no")}</Badge>
            ))}
          </div>
        </div>
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">{t("backupContract")}</strong>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={capabilities.backup_support.before_write ? "secondary" : "destructive"}>
              {t("beforeWrite")}: {capabilities.backup_support.before_write ? t("yes") : t("no")}
            </Badge>
            <Badge variant={capabilities.backup_support.restore ? "secondary" : "destructive"}>
              {t("restore")}: {capabilities.backup_support.restore ? t("yes") : t("no")}
            </Badge>
            <Badge variant="outline">{t("syncOnly")}: {capabilities.backup_support.sync_only ? t("yes") : t("no")}</Badge>
          </div>
        </div>
        <FidelityRows label={t("importFidelity")} fidelity={capabilities.import_fidelity} />
        <FidelityRows label={t("exportFidelity")} fidelity={capabilities.export_fidelity} />
        <div className="grid gap-3 py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">{t("activityCoverage")}</strong>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={capabilities.activity_support.hook_events ? "secondary" : "outline"}>
              {t("hookEvents")}: {capabilities.activity_support.hook_events ? t("yes") : t("no")}
            </Badge>
            <Badge variant={capabilities.activity_support.runtime_endpoint ? "secondary" : "outline"}>
              {t("runtimeEndpoint")}: {capabilities.activity_support.runtime_endpoint ? t("yes") : t("no")}
            </Badge>
            <Badge variant={capabilities.activity_support.session_activity ? "secondary" : "outline"}>
              {t("sessionActivity")}: {capabilities.activity_support.session_activity ? t("yes") : t("no")}
            </Badge>
          </div>
        </div>
      </div>
    </>
  );
}

function ProviderItemsBlock({
  provider,
  workspace,
  onToggle,
  onRequestRun,
  pendingKey,
}: {
  provider: AgentManagementEntry;
  workspace: string | null | undefined;
  onToggle: (setting: ProviderSettingItem, checked: boolean) => void;
  onRequestRun: (setting: ProviderSettingItem) => void;
  pendingKey: string | null;
}) {
  const { t } = useI18n();
  const items = providerSettings(provider);

  return (
    <DetailSection
      title={t("providerItems")}
      description={t("providerItemsDescription")}
    >
      {items.length ? (
        <div className="flex flex-col">
          {items.map((setting) => {
            const key = `${provider.provider_id}:${setting.id}`;
            const pending = pendingKey === key;
            if (setting.kind === "toggle") {
              const checked = setting.value === true;
              return (
                <EntityRow
                  key={setting.id}
                  variant="inline"
                  actions={(
                    <div className="flex items-center gap-3">
                      <span className="text-muted-foreground text-xs">{checked ? t("enabled") : t("disabled")}</span>
                      <Switch checked={checked} disabled={pending} onCheckedChange={(next) => onToggle(setting, next)} />
                    </div>
                  )}
                >
                  <div className="flex min-w-0 flex-col gap-1">
                    <strong className="text-sm font-medium">{settingLabel(setting, t)}</strong>
                    <span className="text-muted-foreground text-sm">{setting.description}</span>
                  </div>
                </EntityRow>
              );
            }
            return (
              <EntityRow
                key={setting.id}
                variant="inline"
                actions={(
                  <Button type="button" variant="outline" disabled={pending} onClick={() => onRequestRun(setting)}>
                    {pending ? <Spinner data-icon="inline-start" /> : <PlayIcon data-icon="inline-start" />}
                    {pending ? t("running") : settingLabel(setting, t)}
                  </Button>
                )}
              >
                <div className="flex min-w-0 flex-col gap-1">
                  <strong className="text-sm font-medium">{settingLabel(setting, t)}</strong>
                  <span className="text-muted-foreground text-sm">{setting.description}</span>
                </div>
              </EntityRow>
            );
          })}
        </div>
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>{t("noProviderItems")}</EmptyTitle>
            <EmptyDescription>{t("noProviderItemsDescription")}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      <div className="text-muted-foreground text-xs">{t("actionsRunWithWorkspace", { workspace: workspace || "-" })}</div>
    </DetailSection>
  );
}

function ProviderDetail({
  provider,
  capabilities,
  isLoading,
  workspace,
}: {
  provider: AgentManagementEntry | undefined;
  capabilities: ProviderCapabilities | undefined;
  isLoading: boolean;
  workspace: string | null | undefined;
}) {
  const { t } = useI18n();
  const detectAgent = useDetectAgent();
  const updateSetting = useUpdateProviderSetting();
  const runSetting = useRunProviderSetting();
  const [confirmAction, setConfirmAction] = useState<ProviderSettingItem | null>(null);
  const [actionResult, setActionResult] = useState<{ title: string; providerId: string; result: unknown } | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<DetailTab>("overview");

  if (isLoading && !provider) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-44 w-full" />
        <Skeleton className="h-44 w-full" />
      </div>
    );
  }

  if (!provider) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>{t("selectProvider")}</EmptyTitle>
          <EmptyDescription>{t("selectProviderDescription")}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  const pendingKey = updateSetting.isPending
    ? `${updateSetting.variables?.provider}:${updateSetting.variables?.settingId}`
    : runSetting.isPending
      ? `${runSetting.variables?.provider}:${runSetting.variables?.settingId}`
      : null;

  function handleToggle(setting: ProviderSettingItem, checked: boolean) {
    updateSetting.mutate({ provider: provider!.provider_id, settingId: setting.id, value: checked });
  }

  function handleRequestRun(setting: ProviderSettingItem) {
    setConfirmAction(setting);
  }

  function handleConfirmRun() {
    if (!confirmAction) return;
    const setting = confirmAction;
    setConfirmAction(null);
    runSetting.mutate(
      { provider: provider!.provider_id, settingId: setting.id, workspace },
      {
        onSuccess: (output) => {
          setActionResult({
            title: settingLabel(setting, t),
            providerId: provider!.provider_id,
            result: output,
          });
        },
      },
    );
  }

  return (
    <>
      <ScrollPane className="min-h-0 h-full" innerClassName="flex flex-col gap-6 pb-2">
        <header className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <ProviderLogo
                providerId={provider.provider_id}
                size="sm"
                alt={provider.name || provider.provider_id}
              />
              <strong className="truncate text-lg font-semibold">{provider.name}</strong>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Dialog open={detailsOpen} onOpenChange={setDetailsOpen}>
                <Button type="button" variant="outline" size="sm" onClick={() => setDetailsOpen(true)}>
                  <InfoIcon data-icon="inline-start" />
                  {t("agentDetails")}
                </Button>
                <DialogContent
                  className="flex h-[min(85dvh,840px)] w-[min(960px,calc(100vw-2rem))] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl"
                  data-agent-capability-details
                >
                  <DialogHeader className="shrink-0 border-b px-6 py-4">
                    <DialogTitle>{t("sessionManagementCapability")}</DialogTitle>
                    <DialogDescription>{t("capabilityDescription")}</DialogDescription>
                  </DialogHeader>
                  <ScrollPane className="min-h-0 flex-1" innerClassName="px-6 py-4">
                    <CapabilityContent capabilities={capabilities} />
                  </ScrollPane>
                </DialogContent>
              </Dialog>
              <Button type="button" variant="outline" size="sm" disabled={detectAgent.isPending} onClick={() => detectAgent.mutate(provider.provider_id)}>
                {detectAgent.isPending ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
                {t("detect")}
              </Button>
            </div>
          </header>
          {detectAgent.error ? <PageError title={t("detectFailed")} message={detectAgent.error.message} /> : null}
          {updateSetting.error ? <PageError title={t("settingUpdateFailed")} message={updateSetting.error.message} /> : null}
          {runSetting.error ? <PageError title={t("providerActionFailed")} message={runSetting.error.message} /> : null}
          <div className="flex flex-wrap gap-1 border-b" role="tablist" aria-label={t("agentCapabilities")}>
            {([
              ["overview", t("overview")],
              ...(provider.capabilities.hook_management ? [["hooks", t("hooks")]] : []),
              ...(provider.capabilities.mcp_management ? [["mcp", "MCP"]] : []),
              ...(provider.capabilities.plugin_management ? [["plugins", t("plugins")]] : []),
              ...(provider.capabilities.config_views.length > 0 ? [["config", t("config")]] : []),
            ] as Array<[DetailTab, string]>).map(([tab, label]) => (
              <button
                key={tab}
                type="button"
                role="tab"
                aria-selected={activeTab === tab}
                onClick={() => setActiveTab(tab)}
                className={cn(
                  "border-b-2 px-3 py-2 text-sm font-medium transition-colors",
                  activeTab === tab ? "border-primary text-foreground" : "border-transparent text-muted-foreground",
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {activeTab === "overview" ? (
            <>
              <EnvironmentBlock provider={provider} />
              <ProviderItemsBlock
                provider={provider}
                workspace={workspace}
                onToggle={handleToggle}
                onRequestRun={handleRequestRun}
                pendingKey={pendingKey}
              />
            </>
          ) : null}
          {activeTab === "hooks" ? <HooksBlock provider={provider} /> : null}
          {activeTab === "mcp" ? <ConfigViewsBlock provider={provider} viewFilter={(view) => view.id === "view_mcp"} /> : null}
          {activeTab === "plugins" ? <ConfigViewsBlock provider={provider} viewFilter={(view) => view.id === "view_plugins"} /> : null}
          {activeTab === "config" ? (
            <ConfigViewsBlock
              provider={provider}
              viewFilter={(view) => view.id !== "view_mcp" && view.id !== "view_plugins"}
            />
          ) : null}
      </ScrollPane>

      <Dialog open={Boolean(confirmAction)} onOpenChange={(open) => !open && setConfirmAction(null)}>
        <DialogContent className="sm:max-w-lg" data-agent-action-confirm>
          <DialogHeader>
            <DialogTitle>{confirmAction ? settingLabel(confirmAction, t) : t("confirmAction")}</DialogTitle>
            <DialogDescription>{confirmAction?.description}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2 rounded-md border p-3 font-mono text-xs">
            <span>{t("provider")}: {provider.provider_id}</span>
            <span>{t("workspace")}: {workspaceName(workspace)}</span>
            {workspace ? <span className="text-muted-foreground break-all">{workspace}</span> : null}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setConfirmAction(null)} disabled={runSetting.isPending}>
              {t("cancel")}
            </Button>
            <Button type="button" onClick={handleConfirmRun} disabled={runSetting.isPending}>
              {t("confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(actionResult)} onOpenChange={(open) => !open && setActionResult(null)}>
        <DialogContent className="sm:max-w-2xl" data-agent-action-result>
          <DialogHeader>
            <DialogTitle>{actionResult?.title}</DialogTitle>
            <DialogDescription>{t("workspaceRepairCompleted")}</DialogDescription>
          </DialogHeader>
          {actionResult ? (
            <ScrollPane className="max-h-[min(70vh,32rem)]">
              <AgentActionResultPanel providerId={actionResult.providerId} result={actionResult.result} />
            </ScrollPane>
          ) : null}
          <DialogFooter>
            <Button type="button" onClick={() => setActionResult(null)}>
              {t("close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function AgentsPage() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const summary = useAgentsSummary();
  const meta = useAgentsMeta();
  const catalog = useAgentProviderCatalog(meta.data?.selected_workspace);
  const providers = summary.data?.providers ?? [];
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const selected = providers.some((provider) => provider.provider_id === selectedProvider)
    ? selectedProvider
    : providers[0]?.provider_id || null;
  const detail = useAgent(selected);
  const workspace = meta.data?.selected_workspace || null;
  const capabilities = catalog.data?.providers.find((provider) => provider.provider_id === selected)?.capability_set;

  if (summary.isLoading || meta.isLoading) return <PageSkeleton />;
  if (summary.error) return <PageError title={t("agentsLoadFailed")} message={summary.error.message} />;
  if (meta.error) return <PageError title={t("workspaceMetadataLoadFailed")} message={meta.error.message} />;
  if (catalog.error) return <PageError title={t("providerCapabilitiesLoadFailed")} message={catalog.error.message} />;

  return (
    <TwoPanePage>
      <PanelCard>
        <section className="flex flex-col gap-3 border-b pb-4">
          <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
        </section>
        <ProviderList
          providers={providers}
          selectedProvider={selected}
          onSelect={setSelectedProvider}
          onPrefetch={(provider) => void prefetchAgent(queryClient, provider)}
        />
      </PanelCard>

      <PanelCard
        variant="plain"
        className={cn("grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4", detail.isFetching && detail.data ? "opacity-95" : "")}
        data-agent-detail-panel
      >
        <AgentStatsStrip provider={detail.data} loading={detail.isLoading} />
        <Separator />
        {detail.error ? <PageError title={t("agentDetailLoadFailed")} message={detail.error.message} /> : null}
        <ProviderDetail
          key={detail.data?.provider_id ?? "empty"}
          provider={detail.data}
          capabilities={capabilities}
          isLoading={detail.isLoading || catalog.isLoading}
          workspace={workspace}
        />
      </PanelCard>
      <Separator className="hidden" />
    </TwoPanePage>
  );
}
