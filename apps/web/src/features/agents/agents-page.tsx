import { useMemo, useState, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ArrowRightIcon, PlayIcon, RefreshCwIcon } from "lucide-react";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { formatBytes, formatExecutableVersion } from "@/lib/format";
import { cn } from "@/lib/utils";
import type {
  AgentEnvironmentStatus,
  AgentManagementEntry,
  ProviderCapabilities,
  ProviderContentFidelity,
  ProviderHookDiagnosisAggregate,
  ProviderSettingItem,
} from "@/lib/types";
import {
  useAgent,
  useAgentProviderCatalog,
  useAgentsMeta,
  useAgentsSummary,
  useDetectAgent,
  useRunProviderSetting,
  useUpdateProviderSetting,
} from "@/features/agents/queries";
import { useManagerStats } from "@/features/manager/queries";
import { AgentActionResultPanel } from "@/features/agents/agent-action-result";

const HOOK_SETTING_IDS = new Set(["install_hook", "verify_hook", "repair_hook", "uninstall_hook"]);

function environmentOf(provider: AgentManagementEntry): AgentEnvironmentStatus {
  return provider.environment || {
    installed: !!provider.installed,
    executable_path: provider.executable_path || null,
    executable_dir: provider.executable_dir || null,
    config_path: provider.config_path || "",
    install_method: provider.install_method || "unknown",
  };
}

function providerSettings(provider: AgentManagementEntry) {
  return (provider.settings || []).filter(
    (setting) => (setting.kind === "toggle" || setting.kind === "action") && !HOOK_SETTING_IDS.has(setting.id),
  );
}

function settingLabel(setting: ProviderSettingItem) {
  if (setting.id === "repair_workspace_sessions") return "Repair current workspace sessions";
  if (setting.id === "show_subagents") return "Show subagents";
  return setting.title || setting.id;
}

function attentionCount(diagnosis: ProviderHookDiagnosisAggregate | undefined) {
  if (!diagnosis) return 0;
  return (
    Number(diagnosis.hook_needs_attention || 0) +
    Number(diagnosis.no_session_match || 0) +
    Number(diagnosis.no_active_runtime || 0) +
    Number(diagnosis.no_events_yet || 0) +
    Number(diagnosis.hook_not_installed || 0)
  );
}

function DetailSection({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-4 border-t pt-5">
      <SectionHeading title={title} description={description} actions={actions} className="border-b-0 pb-0" />
      {children}
    </section>
  );
}

function DetailRow({ label, value, hint }: { label: string; value: string | number | null | undefined; hint?: string }) {
  return (
    <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-1">
        <strong className="text-sm font-medium">{label}</strong>
        {hint ? <span className="text-muted-foreground text-xs">{hint}</span> : null}
      </div>
      <div className="text-muted-foreground min-w-0 break-words font-mono text-xs">{value || "-"}</div>
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
}: {
  providers: AgentManagementEntry[];
  selectedProvider: string | null;
  onSelect: (provider: string) => void;
}) {
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
          <EmptyTitle>No providers</EmptyTitle>
          <EmptyDescription>No agent providers were returned by the backend.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollArea className="min-h-0 flex-1 pr-3">
      <div className="flex flex-col gap-2">
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
            />
          );
        })}
      </div>
    </ScrollArea>
  );
}

function EnvironmentBlock({ provider }: { provider: AgentManagementEntry }) {
  const environment = environmentOf(provider);
  return (
    <DetailSection title="Agent Management Environment">
      <div className="flex flex-col">
        <DetailRow label="Executable path" value={environment.executable_path} />
        <DetailRow label="Executable dir" value={environment.executable_dir} />
        <DetailRow label="Executable version" value={formatExecutableVersion(environment.executable_version)} />
        <DetailRow label="Config path" value={environment.config_path} />
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
  const navigate = useNavigate();
  const providerId = provider?.provider_id;
  const filter = useMemo(() => ({ providers: providerId ? [providerId] : [] }), [providerId]);
  const stats = useManagerStats(filter, { enabled: !!providerId });
  const environment = provider ? environmentOf(provider) : null;
  const sessionCount = provider?.hook_diagnosis?.total_sessions ?? 0;
  const version = formatExecutableVersion(environment?.executable_version);
  const placeholder = loading || stats.isLoading ? <Skeleton className="h-5 w-20" /> : "-";

  return (
    <MetricGrid columns="four" data-agent-stats>
      <MetricTile
        label="Sessions"
        value={provider ? sessionCount : placeholder}
        hint="all workspaces"
        variant="compact"
        onClick={() => {
          if (!providerId) return;
          navigate(`/manager?provider=${encodeURIComponent(providerId)}&view=sessions`);
        }}
      />
      <MetricTile
        label="Size"
        value={stats.data ? formatBytes(stats.data.all_workspace_size_bytes) : placeholder}
        hint="indexed storage"
        variant="compact"
      />
      <MetricTile
        label="Version"
        value={version || placeholder}
        hint="executable"
        variant="compact"
        title={version ?? undefined}
      />
      <MetricTile
        label="Install Method"
        value={environment?.install_method || placeholder}
        variant="compact"
      />
    </MetricGrid>
  );
}

function HooksBlock({ provider }: { provider: AgentManagementEntry }) {
  const hook = provider.hook || {};
  const diagnosis = provider.hook_diagnosis || {};
  const version = hook.installed_version && hook.current_version && hook.installed_version !== hook.current_version
    ? `${hook.installed_version} -> ${hook.current_version}`
    : hook.installed_version || hook.current_version || "-";

  return (
    <DetailSection
      title="Hooks"
      description="Provider hook install, runtime, and session diagnosis summary."
      actions={(
        <Button asChild variant="outline">
          <Link to="/hooks">
            Open Hooks
            <ArrowRightIcon data-icon="inline-end" />
          </Link>
        </Button>
      )}
    >
      <SummaryGrid
        items={[
          { label: "Hook status", value: hook.status || "unsupported" },
          { label: "Version", value: version },
          { label: "Sessions", value: diagnosis.total_sessions || 0 },
          { label: "Active runtime", value: diagnosis.active_runtime_sessions || 0 },
          { label: "Attention", value: attentionCount(diagnosis) },
        ]}
      />
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
        )) : <span className="text-xs text-muted-foreground">Unknown</span>}
      </div>
    </div>
  );
}

function CapabilityBlock({ capabilities }: { capabilities: ProviderCapabilities | undefined }) {
  if (!capabilities) {
    return (
      <DetailSection title="Session Management Capability">
        <div className="text-sm text-muted-foreground">Capability catalog data is unavailable.</div>
      </DetailSection>
    );
  }

  const operations = [
    ["Scan", capabilities.scan],
    ["Import", capabilities.import],
    ["Export", capabilities.export],
    ["Delete", capabilities.delete],
    ["Rename", capabilities.rename],
    ["Resume", capabilities.resume],
  ] as const;
  const topology = [
    ["Multiple files", capabilities.write_risk.multiple_files],
    ["SQLite", capabilities.write_risk.sqlite],
    ["Sidecars", capabilities.write_risk.sidecar_files],
    ["Index repair", capabilities.write_risk.index_repair],
  ] as const;

  return (
    <DetailSection
      title="Session Management Capability"
      description="Projection quality and native write risk declared by the provider implementation."
    >
      <SummaryGrid
        items={[
          { label: "Storage", value: capabilityLabel(capabilities.storage_shape) },
          { label: "Scan", value: capabilityLabel(capabilities.scan_strategy) },
          { label: "Paging", value: capabilityLabel(capabilities.page_strategy) },
          { label: "Turn quality", value: capabilityLabel(capabilities.turn_quality) },
          { label: "Resume", value: capabilityLabel(capabilities.resume_quality) },
          { label: "Write risk", value: capabilityLabel(capabilities.write_risk.level) },
        ]}
      />
      <div className="flex flex-col">
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">Operations</strong>
          <div className="flex flex-wrap gap-1.5">
            {operations.map(([label, supported]) => (
              <Badge key={label} variant={supported ? "secondary" : "outline"}>{label}: {supported ? "yes" : "no"}</Badge>
            ))}
          </div>
        </div>
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <div className="flex min-w-0 flex-col gap-1">
            <strong className="text-sm font-medium">Native write risk</strong>
            <span className="text-xs text-muted-foreground">Storage planes touched by delete, rename, import, or sync.</span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={riskVariant(capabilities.write_risk.level)}>{capabilityLabel(capabilities.write_risk.level)}</Badge>
            {topology.map(([label, present]) => (
              <Badge key={label} variant={present ? "outline" : "ghost"}>{label}: {present ? "yes" : "no"}</Badge>
            ))}
          </div>
        </div>
        <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">Backup contract</strong>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={capabilities.backup_support.before_write ? "secondary" : "destructive"}>
              Before write: {capabilities.backup_support.before_write ? "yes" : "no"}
            </Badge>
            <Badge variant={capabilities.backup_support.restore ? "secondary" : "destructive"}>
              Restore: {capabilities.backup_support.restore ? "yes" : "no"}
            </Badge>
            <Badge variant="outline">Sync only: {capabilities.backup_support.sync_only ? "yes" : "no"}</Badge>
          </div>
        </div>
        <FidelityRows label="Import fidelity" fidelity={capabilities.import_fidelity} />
        <FidelityRows label="Export fidelity" fidelity={capabilities.export_fidelity} />
        <div className="grid gap-3 py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
          <strong className="text-sm font-medium">Activity coverage</strong>
          <div className="flex flex-wrap gap-1.5">
            <Badge variant={capabilities.activity_support.hook_events ? "secondary" : "outline"}>
              Hook events: {capabilities.activity_support.hook_events ? "yes" : "no"}
            </Badge>
            <Badge variant={capabilities.activity_support.runtime_endpoint ? "secondary" : "outline"}>
              Runtime endpoint: {capabilities.activity_support.runtime_endpoint ? "yes" : "no"}
            </Badge>
            <Badge variant={capabilities.activity_support.session_activity ? "secondary" : "outline"}>
              Session activity: {capabilities.activity_support.session_activity ? "yes" : "no"}
            </Badge>
          </div>
        </div>
      </div>
    </DetailSection>
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
  const items = providerSettings(provider);

  return (
    <DetailSection
      title="Agent Provider Items"
      description="Provider-specific toggles and repair actions."
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
                      <span className="text-muted-foreground text-xs">{checked ? "Enabled" : "Disabled"}</span>
                      <Switch checked={checked} disabled={pending} onCheckedChange={(next) => onToggle(setting, next)} />
                    </div>
                  )}
                >
                  <div className="flex min-w-0 flex-col gap-1">
                    <strong className="text-sm font-medium">{settingLabel(setting)}</strong>
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
                    {pending ? "Running" : settingLabel(setting)}
                  </Button>
                )}
              >
                <div className="flex min-w-0 flex-col gap-1">
                  <strong className="text-sm font-medium">{settingLabel(setting)}</strong>
                  <span className="text-muted-foreground text-sm">{setting.description}</span>
                </div>
              </EntityRow>
            );
          })}
        </div>
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>No provider items</EmptyTitle>
            <EmptyDescription>This provider has no non-hook controls.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      <div className="text-muted-foreground text-xs">Actions run with workspace {workspace || "-"}.</div>
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
  const detectAgent = useDetectAgent();
  const updateSetting = useUpdateProviderSetting();
  const runSetting = useRunProviderSetting();
  const [confirmAction, setConfirmAction] = useState<ProviderSettingItem | null>(null);
  const [actionResult, setActionResult] = useState<{ title: string; providerId: string; result: unknown } | null>(null);

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
          <EmptyTitle>Select a provider</EmptyTitle>
          <EmptyDescription>Choose an agent provider on the left to inspect its environment and controls.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  const environment = environmentOf(provider);
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
            title: settingLabel(setting),
            providerId: provider!.provider_id,
            result: output,
          });
        },
      },
    );
  }

  return (
    <>
      <ScrollArea className="min-h-0 h-full pr-3">
      <div className="flex flex-col gap-6 pb-2">
        <header className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <ProviderLogo
                providerId={provider.provider_id}
                size="sm"
                alt={provider.name || provider.provider_id}
              />
              <strong className="truncate text-lg font-semibold">{provider.name}</strong>
            </div>
            <Button type="button" variant="outline" disabled={detectAgent.isPending} onClick={() => detectAgent.mutate(provider.provider_id)}>
              {detectAgent.isPending ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
              Detect
            </Button>
          </header>
          {detectAgent.error ? <PageError title="Detect failed" message={detectAgent.error.message} /> : null}
          {updateSetting.error ? <PageError title="Setting update failed" message={updateSetting.error.message} /> : null}
          {runSetting.error ? <PageError title="Provider action failed" message={runSetting.error.message} /> : null}
          <EnvironmentBlock provider={provider} />
          <CapabilityBlock capabilities={capabilities} />
          <HooksBlock provider={provider} />
          <ProviderItemsBlock
            provider={provider}
            workspace={workspace}
            onToggle={handleToggle}
            onRequestRun={handleRequestRun}
            pendingKey={pendingKey}
          />
        </div>
      </ScrollArea>

      <Dialog open={Boolean(confirmAction)} onOpenChange={(open) => !open && setConfirmAction(null)}>
        <DialogContent className="sm:max-w-lg" data-agent-action-confirm>
          <DialogHeader>
            <DialogTitle>{confirmAction ? settingLabel(confirmAction) : "Confirm action"}</DialogTitle>
            <DialogDescription>{confirmAction?.description}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2 rounded-md border p-3 font-mono text-xs">
            <span>Provider: {provider.provider_id}</span>
            <span>Workspace: {workspaceName(workspace)}</span>
            {workspace ? <span className="text-muted-foreground break-all">{workspace}</span> : null}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setConfirmAction(null)} disabled={runSetting.isPending}>
              Cancel
            </Button>
            <Button type="button" onClick={handleConfirmRun} disabled={runSetting.isPending}>
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(actionResult)} onOpenChange={(open) => !open && setActionResult(null)}>
        <DialogContent className="sm:max-w-2xl" data-agent-action-result>
          <DialogHeader>
            <DialogTitle>{actionResult?.title}</DialogTitle>
            <DialogDescription>Workspace session repair completed.</DialogDescription>
          </DialogHeader>
          {actionResult ? (
            <ScrollArea className="max-h-[min(70vh,32rem)] pr-3">
              <AgentActionResultPanel providerId={actionResult.providerId} result={actionResult.result} />
            </ScrollArea>
          ) : null}
          <DialogFooter>
            <Button type="button" onClick={() => setActionResult(null)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function AgentsPage() {
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
  if (summary.error) return <PageError title="Agents failed to load" message={summary.error.message} />;
  if (meta.error) return <PageError title="Workspace metadata failed to load" message={meta.error.message} />;
  if (catalog.error) return <PageError title="Provider capabilities failed to load" message={catalog.error.message} />;

  return (
    <TwoPanePage>
      <PanelCard>
        <section className="flex flex-col gap-3 border-b pb-4">
          <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
        </section>
        <ProviderList providers={providers} selectedProvider={selected} onSelect={setSelectedProvider} />
      </PanelCard>

      <PanelCard
        variant="plain"
        className={cn("grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4", detail.isFetching && detail.data ? "opacity-95" : "")}
        data-agent-detail-panel
      >
        <AgentStatsStrip provider={detail.data} loading={detail.isLoading} />
        <Separator />
        {detail.error ? <PageError title="Agent detail failed to load" message={detail.error.message} /> : null}
        <ProviderDetail
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
