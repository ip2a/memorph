import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRightIcon, PlayIcon, RefreshCwIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PanelCard } from "@/components/shared/panel-card";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import {
  providerListInstallStatus,
  ProviderListInstallStatusBadge,
  ProviderListStatusTrailing,
} from "@/components/shared/provider-list-status";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type { AgentEnvironmentStatus, AgentManagementEntry, ProviderHookDiagnosisAggregate, ProviderSettingItem } from "@/lib/types";
import {
  useAgent,
  useAgentsMeta,
  useAgentsSummary,
  useDetectAgent,
  useRunProviderSetting,
  useUpdateProviderSetting,
} from "@/features/agents/queries";

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
    <Card size="sm">
      <CardHeader>
        <CardTitle>Agent Management Environment</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <SummaryGrid
          items={[
            { label: "Install status", value: environment.installed ? "Installed" : "Not detected" },
            { label: "Install method", value: environment.install_method || "unknown" },
          ]}
        />
        <div className="flex flex-col">
          <DetailRow label="Executable path" value={environment.executable_path} />
          <DetailRow label="Executable dir" value={environment.executable_dir} />
          <DetailRow label="Executable version" value={environment.executable_version} />
          <DetailRow label="Config path" value={environment.config_path} />
        </div>
      </CardContent>
    </Card>
  );
}

function HooksBlock({ provider }: { provider: AgentManagementEntry }) {
  const hook = provider.hook || {};
  const diagnosis = provider.hook_diagnosis || {};
  const events = provider.hook_profile?.events || [];
  const version = hook.installed_version && hook.current_version && hook.installed_version !== hook.current_version
    ? `${hook.installed_version} -> ${hook.current_version}`
    : hook.installed_version || hook.current_version || "-";

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Hooks</CardTitle>
        <CardDescription>Provider hook install, runtime, and session diagnosis summary.</CardDescription>
        <CardAction>
          <Button asChild variant="outline">
            <Link to="/hooks">
              Open Hooks
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <SummaryGrid
          items={[
            { label: "Hook status", value: hook.status || "unsupported" },
            { label: "Version", value: version },
            { label: "Sessions", value: diagnosis.total_sessions || 0 },
            { label: "Active runtime", value: diagnosis.active_runtime_sessions || 0 },
            { label: "Attention", value: attentionCount(diagnosis) },
          ]}
        />
        {events.length ? (
          <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
            <strong className="text-sm font-medium">Hook events</strong>
            <div className="flex flex-wrap gap-2">
              {events.map((event) => (
                <Badge key={`${event.name}-${String(event.blocking)}`} variant="outline">
                  {event.name}{event.blocking ? " *" : ""}
                </Badge>
              ))}
            </div>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ProviderItemsBlock({
  provider,
  workspace,
  onToggle,
  onRun,
  pendingKey,
  result,
}: {
  provider: AgentManagementEntry;
  workspace: string | null | undefined;
  onToggle: (setting: ProviderSettingItem, checked: boolean) => void;
  onRun: (setting: ProviderSettingItem) => void;
  pendingKey: string | null;
  result: unknown;
}) {
  const items = providerSettings(provider);
  const actionItems = items.filter((setting) => setting.kind === "action");

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Agent Provider Items</CardTitle>
        <CardDescription>Provider-specific toggles and repair actions.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
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
                    <Button type="button" variant="outline" disabled={pending} onClick={() => onRun(setting)}>
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
        {actionItems.length && result ? (
          <pre className="bg-muted text-muted-foreground max-h-80 overflow-auto rounded-md p-3 text-xs">
            {JSON.stringify(result, null, 2)}
          </pre>
        ) : null}
        <div className="text-muted-foreground text-xs">Actions run with workspace {workspace || "-"}.</div>
      </CardContent>
    </Card>
  );
}

function ProviderDetail({
  provider,
  isLoading,
  workspace,
}: {
  provider: AgentManagementEntry | undefined;
  isLoading: boolean;
  workspace: string | null | undefined;
}) {
  const detectAgent = useDetectAgent();
  const updateSetting = useUpdateProviderSetting();
  const runSetting = useRunProviderSetting();
  const [lastResult, setLastResult] = useState<unknown>(null);

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

  function handleRun(setting: ProviderSettingItem) {
    runSetting.mutate(
      { provider: provider!.provider_id, settingId: setting.id, workspace },
      { onSuccess: (output) => setLastResult(output) },
    );
  }

  return (
    <ScrollArea className="h-full pr-3">
      <div className="flex flex-col gap-4">
        <header className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-2">
            <strong className="truncate text-lg font-semibold">{provider.name}</strong>
            <div className="flex flex-wrap gap-2">
              <Badge variant="outline">{provider.provider_id}</Badge>
              <ProviderListInstallStatusBadge status={providerListInstallStatus(provider, "agent")} />
              <Badge variant="outline">{environment.install_method || "unknown"}</Badge>
            </div>
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
        <HooksBlock provider={provider} />
        <ProviderItemsBlock
          provider={provider}
          workspace={workspace}
          onToggle={handleToggle}
          onRun={handleRun}
          pendingKey={pendingKey}
          result={lastResult}
        />
      </div>
    </ScrollArea>
  );
}

export function AgentsPage() {
  const summary = useAgentsSummary();
  const meta = useAgentsMeta();
  const providers = summary.data?.providers ?? [];
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const selected = providers.some((provider) => provider.provider_id === selectedProvider)
    ? selectedProvider
    : providers[0]?.provider_id || null;
  const detail = useAgent(selected);
  const workspace = meta.data?.selected_workspace || null;

  if (summary.isLoading || meta.isLoading) return <PageSkeleton />;
  if (summary.error) return <PageError title="Agents failed to load" message={summary.error.message} />;
  if (meta.error) return <PageError title="Workspace metadata failed to load" message={meta.error.message} />;

  return (
    <TwoPanePage>
      <PanelCard>
        <section className="flex flex-col gap-3 border-b pb-4">
          <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
        </section>
        <ProviderList providers={providers} selectedProvider={selected} onSelect={setSelectedProvider} />
      </PanelCard>

      <PanelCard variant="plain" className={cn(detail.isFetching && detail.data ? "opacity-95" : "")}>
        {detail.error ? <PageError title="Agent detail failed to load" message={detail.error.message} /> : null}
        <ProviderDetail provider={detail.data} isLoading={detail.isLoading} workspace={workspace} />
      </PanelCard>
      <Separator className="hidden" />
    </TwoPanePage>
  );
}
