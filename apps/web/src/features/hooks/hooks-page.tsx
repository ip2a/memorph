import { useMemo, useState } from "react";
import { WrenchIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PanelCard } from "@/components/shared/panel-card";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import {
  providerHookAttention,
  providerListInstallStatus,
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
} from "@/lib/types";
import { useHookProviderOverview, useHooksMeta, useHooksOverview, useRunHookProviderOperation } from "@/features/hooks/queries";

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
  return <MetricTile label={label} value={value ?? "-"} />;
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

function DetailCard({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
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
  const hook = provider.hook || {};
  const profileEvents = provider.hook_profile?.events || [];
  const requiredEvents = provider.hook_required_events || [];
  const pendingOperation = runOperation.isPending ? runOperation.variables?.operation : null;

  return (
    <ScrollArea className="h-full pr-3">
      <div className="flex flex-col gap-4">
        <header className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-2">
            <strong className="truncate text-lg font-semibold">{providerName(provider)}</strong>
            <small className="text-muted-foreground">Hook provider detail, diagnostics, and operations.</small>
            <div className="flex flex-wrap gap-2">
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

        <Card size="sm">
          <CardHeader>
            <CardTitle>Hook Summary</CardTitle>
            <CardDescription>Provider hook status, event requirements, and session diagnosis.</CardDescription>
            <CardAction>{hook.message ? <Badge variant="outline">{hook.message}</Badge> : null}</CardAction>
          </CardHeader>
          <CardContent>
            <MetricGrid>
              <SummaryItem label="Hook status" value={hook.status || "unknown"} />
              <SummaryItem label="Required events" value={requiredEvents.length || profileEvents.length} />
              <SummaryItem label="Last event" value={formatDateTime(hook.last_event_at)} />
              <SummaryItem label="Linked sessions" value={provider.hook_diagnosis?.linked || 0} />
              <SummaryItem label="Weak sessions" value={provider.hook_diagnosis?.weakly_linked || 0} />
              <SummaryItem label="No match" value={provider.hook_diagnosis?.no_session_match || 0} />
              <SummaryItem label="Active runtime" value={detail.runtime_sessions.length} />
              <SummaryItem label="Recent errors" value={detail.recent_errors.length} />
            </MetricGrid>
          </CardContent>
        </Card>

        <DetailCard title="Hook Event Profile" description="Provider events that memorph records or blocks for runtime correlation.">
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
        </DetailCard>

        <DetailCard title="Runtime Sessions" description="Active or recently observed hook runtime sessions for this provider.">
          <RuntimeRows sessions={detail.runtime_sessions} />
        </DetailCard>

        <DetailCard title="Recent Events" description="Latest hook events recorded for this provider.">
          <EventRows events={detail.recent_events} />
        </DetailCard>

        <DetailCard title="Recent Errors" description="Latest hook errors that mention this provider.">
          <ErrorRows errors={detail.recent_errors} />
        </DetailCard>
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

  const summary = overview.data?.summary;

  return (
    <TwoPanePage>
      <PanelCard>
        <section className="flex flex-col gap-3 border-b pb-4">
          <WorkspaceIdentity workspace={workspace} titleClassName="mt-1 block text-lg leading-tight" pathClassName="mt-1" />
          <MetricGrid columns="two">
            <SummaryItem label="Providers" value={summary?.providers || 0} />
            <SummaryItem label="Attention" value={summary?.needs_attention || 0} />
            <SummaryItem label="Runtime" value={summary?.active_runtime_sessions || 0} />
            <SummaryItem label="Errors" value={summary?.recent_errors || 0} />
          </MetricGrid>
        </section>
        <HooksProviderList providers={providers} selectedProvider={selected} onSelect={setSelectedProvider} />
      </PanelCard>

      <PanelCard variant="plain" className={cn(detail.isFetching && detail.data ? "opacity-95" : "")}>
        {detail.error ? <PageError title="Hook provider detail failed to load" message={detail.error.message} /> : null}
        <ProviderDetail detail={detail.data} isLoading={detail.isLoading} />
      </PanelCard>
    </TwoPanePage>
  );
}
