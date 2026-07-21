import { useMemo, useState, type ReactNode } from "react";
import { Trash2Icon, WrenchIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PanelCard } from "@/components/shared/panel-card";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { SectionHeading } from "@/components/shared/section-heading";
import {
  providerHookAttention,
  providerListInstallStatus,
  ProviderListStatusTrailing,
} from "@/components/shared/provider-list-status";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import { WorkspaceIdentity } from "@/components/shared/workspace-identity";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { formatDateTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import type {
  AgentManagementEntry,
  HookErrorRecord,
  HookEventRecord,
  HookProviderOverviewPayload,
  HookRuntimeSession,
  InstalledHook,
} from "@/lib/types";
import {
  useHookProviderOverview,
  useHooksMeta,
  useHooksOverview,
  useInstalledHooks,
  useRemoveInstalledHook,
  useRunHookProviderOperation,
} from "@/features/hooks/queries";

function providerName(provider: AgentManagementEntry) {
  return provider.name || provider.provider_id;
}

function hookStatus(provider: AgentManagementEntry) {
  return provider.hook?.status || "unknown";
}

function detailStatusBadge(status: string) {
  if (status === "installed_ok") return <Badge variant="secondary">installed_ok</Badge>;
  if (status === "not_installed") return <Badge variant="outline">not_installed</Badge>;
  return <Badge variant="destructive">{status}</Badge>;
}

function operationIds(provider: AgentManagementEntry) {
  const capabilities = provider.hook_capabilities || {};
  const supported = !!provider.hook_profile;
  if (!supported) return [];
  const available = {
    install_hook: capabilities.install !== false,
    verify_hook: capabilities.verify !== false,
    repair_hook: capabilities.repair !== false,
    uninstall_hook: capabilities.uninstall !== false,
  };
  const keepAvailable = (ids: string[]) => ids.filter((id) => available[id as keyof typeof available]);
  const status = hookStatus(provider);
  if (status === "not_installed") return keepAvailable(["install_hook", "verify_hook"]);
  if (
    [
      "installed_disabled",
      "installed_stale_binary",
      "installed_stale_endpoint",
      "installed_broken_config",
      "installed_conflict",
      "repairable",
      "needs_user_action",
    ].includes(status)
  ) {
    return keepAvailable(["repair_hook", "verify_hook", "uninstall_hook"]);
  }
  if (status === "installed_ok") return keepAvailable(["verify_hook", "repair_hook", "uninstall_hook"]);
  return keepAvailable(["verify_hook"]);
}

function operationLabel(operation: string) {
  if (operation === "install_hook") return "Install";
  if (operation === "verify_hook") return "Verify";
  if (operation === "repair_hook") return "Repair";
  if (operation === "uninstall_hook") return "Uninstall";
  return operation;
}

function SummaryItem({ label, value }: { label: string; value: string | number | null | undefined }) {
  return <MetricTile label={label} value={value ?? "-"} variant="compact" />;
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

function HookStatsStrip({
  provider,
  runtimeCount,
  loading,
}: {
  provider: AgentManagementEntry | undefined;
  runtimeCount: number;
  loading: boolean;
}) {
  const hook = provider?.hook || {};
  const diagnosis = provider?.hook_diagnosis || {};
  const version = hook.installed_version && hook.current_version && hook.installed_version !== hook.current_version
    ? `${hook.installed_version} -> ${hook.current_version}`
    : hook.installed_version || hook.current_version || null;
  const placeholder = loading ? <Skeleton className="h-5 w-20" /> : "-";

  return (
    <MetricGrid columns="four" data-hook-stats>
      <MetricTile
        label="Hook Status"
        value={provider ? hook.status || "unknown" : placeholder}
        hint="install state"
        title={hook.message || undefined}
        variant="compact"
      />
      <MetricTile
        label="Sessions"
        value={provider ? diagnosis.total_sessions ?? 0 : placeholder}
        hint="total sessions"
        variant="compact"
      />
      <MetricTile
        label="Active Runtime"
        value={provider ? runtimeCount : placeholder}
        hint="live sessions"
        variant="compact"
      />
      <MetricTile
        label="Version"
        value={version || placeholder}
        hint="hook package"
        variant="compact"
      />
    </MetricGrid>
  );
}

function HooksProviderList({
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
        const leftOk = hookStatus(left) === "installed_ok";
        const rightOk = hookStatus(right) === "installed_ok";
        if (leftOk !== rightOk) return leftOk ? -1 : 1;
        return providerName(left).localeCompare(providerName(right));
      }),
    [providers],
  );

  if (!ordered.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No hook providers</EmptyTitle>
          <EmptyDescription>No providers were returned by the hook overview.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollArea className="min-h-0 flex-1 pr-3">
      <div className="flex flex-col gap-2">
        {ordered.map((provider) => {
          const selected = provider.provider_id === selectedProvider;
          const status = providerListInstallStatus(provider, "hook");
          const attention = providerHookAttention(provider);
          return (
            <SelectableRowButton
              key={provider.provider_id}
              selected={selected}
              leading={<ProviderLogo providerId={provider.provider_id} size="sm" alt={providerName(provider)} />}
              title={providerName(provider)}
              trailing={<ProviderListStatusTrailing attention={attention} status={status} />}
              onClick={() => onSelect(provider.provider_id)}
            />
          );
        })}
      </div>
    </ScrollArea>
  );
}

function EventRows({ events }: { events: HookEventRecord[] }) {
  if (!events.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No recent events</EmptyTitle>
          <EmptyDescription>No hook events were recorded for this provider.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col">
      {events.map((event) => {
        const subject = event.tool?.name || event.message?.role || event.provider_session_id || event.run_id || event.event_id;
        return (
          <EntityRow
            key={event.event_id}
            variant="inline"
            actions={(
              <>
                <Badge variant="outline">{formatDateTime(event.timestamp)}</Badge>
                {event.provider_session_id ? <Badge variant="outline">{event.provider_session_id}</Badge> : null}
              </>
            )}
          >
            <div className="flex min-w-0 flex-col gap-1">
              <strong className="truncate text-sm font-medium">{event.event_type}</strong>
              <span className="text-muted-foreground truncate font-mono text-xs">{String(subject || "-")}</span>
            </div>
          </EntityRow>
        );
      })}
    </div>
  );
}

function RuntimeRows({ sessions }: { sessions: HookRuntimeSession[] }) {
  if (!sessions.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No active runtime</EmptyTitle>
          <EmptyDescription>No hook runtime sessions are currently associated with this provider.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col">
      {sessions.map((session) => (
        <EntityRow
          key={String(session.runtime_id)}
          variant="inline"
          actions={(
            <>
              <Badge variant="outline">{session.status}</Badge>
              {session.current_tool?.name ? <Badge variant="outline">{session.current_tool.name}</Badge> : null}
              <Badge variant="outline">{formatDateTime(session.last_event_at)}</Badge>
            </>
          )}
        >
          <div className="flex min-w-0 flex-col gap-1">
            <strong className="truncate text-sm font-medium">{session.session_title || session.provider_session_id || String(session.runtime_id)}</strong>
            <span className="text-muted-foreground break-all font-mono text-xs">{session.cwd || "-"}</span>
          </div>
        </EntityRow>
      ))}
    </div>
  );
}

function ErrorRows({ errors }: { errors: HookErrorRecord[] }) {
  if (!errors.length) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>No recent errors</EmptyTitle>
          <EmptyDescription>No hook errors were recorded for this provider.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col">
      {errors.map((error) => (
        <EntityRow key={`${error.timestamp}-${error.scope}-${error.message}`} variant="inline" actions={<Badge variant="outline">{formatDateTime(error.timestamp)}</Badge>}>
          <div className="flex min-w-0 flex-col gap-1">
            <strong className="truncate text-sm font-medium">{error.scope}</strong>
            <span className="text-muted-foreground break-words text-sm">{error.message}</span>
          </div>
        </EntityRow>
      ))}
    </div>
  );
}

function ProviderDetail({ detail, isLoading }: { detail: HookProviderOverviewPayload | undefined; isLoading: boolean }) {
  const runOperation = useRunHookProviderOperation();

  if (isLoading && !detail) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-44 w-full" />
        <Skeleton className="h-44 w-full" />
      </div>
    );
  }

  if (!detail) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>Select a provider</EmptyTitle>
          <EmptyDescription>Choose a hook provider on the left to inspect diagnostics and runtime activity.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  const provider = detail.provider;
  const installedHooks = useInstalledHooks(provider.provider_id);
  const removeInstalled = useRemoveInstalledHook();
  const hook = provider.hook || {};
  const profileEvents = provider.hook_profile?.events || [];
  const diagnosis = provider.hook_diagnosis || {};
  const pendingOperation = runOperation.isPending ? runOperation.variables?.operation : null;

  return (
    <ScrollArea className="min-h-0 h-full pr-3">
      <div className="flex flex-col gap-6 pb-2">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2">
            <ProviderLogo
              providerId={provider.provider_id}
              size="sm"
              alt={providerName(provider)}
              className="row-span-3 self-center"
            />
            <strong className="truncate text-lg font-semibold">{providerName(provider)}</strong>
            <small className="col-start-2 text-muted-foreground">Hook provider detail, diagnostics, and operations.</small>
            <div className="col-start-2 flex flex-wrap gap-2">
              <Badge variant="outline">{provider.provider_id}</Badge>
              {detailStatusBadge(hookStatus(provider))}
              {provider.hook_profile ? <Badge variant="outline">supported</Badge> : <Badge variant="outline">unsupported</Badge>}
            </div>
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            {operationIds(provider).map((operation) => (
              <Button
                key={operation}
                type="button"
                variant={operation === "uninstall_hook" ? "destructive" : "outline"}
                disabled={runOperation.isPending}
                onClick={() => runOperation.mutate({ provider: provider.provider_id, operation })}
              >
                {pendingOperation === operation ? <Spinner data-icon="inline-start" /> : <WrenchIcon data-icon="inline-start" />}
                {pendingOperation === operation ? "Running" : operationLabel(operation)}
              </Button>
            ))}
          </div>
        </header>

        {runOperation.error ? <PageError title="Hook operation failed" message={runOperation.error.message} /> : null}
        {runOperation.data?.message ? <PageError title="Hook operation result" message={runOperation.data.message} /> : null}

        <DetailSection
          title="Hook Summary"
          description="Provider hook status, event requirements, and session diagnosis."
        >
          {hook.message ? (
            <p className="text-muted-foreground text-sm break-words">{hook.message}</p>
          ) : null}
          <MetricGrid columns="auto">
            <SummaryItem label="Linked sessions" value={diagnosis.linked || 0} />
            <SummaryItem label="Weak sessions" value={diagnosis.weakly_linked || 0} />
            <SummaryItem label="No match" value={diagnosis.no_session_match || 0} />
            <SummaryItem label="Recent errors" value={detail.recent_errors.length} />
            <SummaryItem label="Last event" value={formatDateTime(hook.last_event_at)} />
            <SummaryItem
              label="Hook version"
              value={
                hook.installed_version && hook.current_version && hook.installed_version !== hook.current_version
                  ? `${hook.installed_version} -> ${hook.current_version}`
                  : hook.installed_version || hook.current_version
              }
            />
          </MetricGrid>
        </DetailSection>


        <DetailSection
          title="Installed Hooks"
          description={installedHooks.data?.config_path ? `All hooks found in ${installedHooks.data.config_path}` : "All hooks found in the provider configuration."}
        >
          {installedHooks.isLoading ? <Spinner /> : null}
          {installedHooks.error ? <PageError title="Installed hooks failed to load" message={installedHooks.error.message} /> : null}
          {installedHooks.data?.hooks.length ? (
            <div className="flex flex-col gap-2">
              {installedHooks.data.hooks.map((item: InstalledHook) => (
                <div key={`${item.event}-${item.index}`} className="flex items-start justify-between gap-3 rounded-md border p-3 text-sm">
                  <div className="min-w-0">
                    <div className="flex flex-wrap gap-2">
                      <Badge variant="outline">{item.event}</Badge>
                      <Badge variant={item.managed_by_memorph ? "secondary" : "outline"}>{item.source}</Badge>
                    </div>
                    <p className="mt-2 break-all text-muted-foreground">{item.command || item.hook_type || "configured hook"}</p>
                  </div>
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    title="Remove hook"
                    disabled={removeInstalled.isPending}
                    onClick={() => removeInstalled.mutate({ provider: provider.provider_id, event: item.event, index: item.index, fingerprint: item.fingerprint })}
                  >
                    <Trash2Icon />
                  </Button>
                </div>
              ))}
            </div>
          ) : !installedHooks.isLoading ? <Empty><EmptyHeader><EmptyTitle>No installed hooks</EmptyTitle></EmptyHeader></Empty> : null}
          {removeInstalled.error ? <PageError title="Remove hook failed" message={removeInstalled.error.message} /> : null}
        </DetailSection>

        <DetailSection title="Hook Event Profile" description="Provider events that memorph records or blocks for runtime correlation.">
          {profileEvents.length ? (
            <div className="flex flex-wrap gap-2">
              {profileEvents.map((event) => (
                <Badge key={`${event.name}-${String(event.blocking)}`} variant="outline">
                  {event.name}{event.blocking ? " *" : ""}
                </Badge>
              ))}
            </div>
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyTitle>No hook profile</EmptyTitle>
                <EmptyDescription>This provider does not expose a managed hook profile.</EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </DetailSection>

        <DetailSection title="Runtime Sessions" description="Active or recently observed hook runtime sessions for this provider.">
          <RuntimeRows sessions={detail.runtime_sessions} />
        </DetailSection>

        <DetailSection title="Recent Events" description="Latest hook events recorded for this provider.">
          <EventRows events={detail.recent_events} />
        </DetailSection>

        <DetailSection title="Recent Errors" description="Latest hook errors that mention this provider.">
          <ErrorRows errors={detail.recent_errors} />
        </DetailSection>
      </div>
    </ScrollArea>
  );
}

export function HooksPage() {
  const overview = useHooksOverview();
  const meta = useHooksMeta();
  const providers = overview.data?.providers ?? [];
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const selected = providers.some((provider) => provider.provider_id === selectedProvider)
    ? selectedProvider
    : providers[0]?.provider_id || null;
  const detail = useHookProviderOverview(selected);
  const workspace = meta.data?.selected_workspace || null;

  if (overview.isLoading || meta.isLoading) return <PageSkeleton />;
  if (overview.error) return <PageError title="Hooks failed to load" message={overview.error.message} />;
  if (meta.error) return <PageError title="Workspace metadata failed to load" message={meta.error.message} />;

  return (
    <TwoPanePage>
      <PanelCard>
        <section className="flex flex-col gap-3 border-b pb-4">
          <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
        </section>
        <HooksProviderList providers={providers} selectedProvider={selected} onSelect={setSelectedProvider} />
      </PanelCard>

      <PanelCard
        variant="plain"
        className={cn("grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)] gap-4", detail.isFetching && detail.data ? "opacity-95" : "")}
        data-hook-detail-panel
      >
        <HookStatsStrip
          provider={detail.data?.provider}
          runtimeCount={detail.data?.runtime_sessions.length ?? 0}
          loading={detail.isLoading}
        />
        <Separator />
        {detail.error ? <PageError title="Hook provider detail failed to load" message={detail.error.message} /> : null}
        <ProviderDetail detail={detail.data} isLoading={detail.isLoading} />
      </PanelCard>
    </TwoPanePage>
  );
}
