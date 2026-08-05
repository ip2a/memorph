import { useState } from "react";
import { LoaderCircle, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { PageError } from "@/components/shared/page-states";
import { useI18n } from "@/lib/i18n-context";
import type {
  AgentManagementEntry,
  ProviderConfigIssue,
  ProviderConfigEntry,
  ProviderConfigRow,
  ProviderConfigTone,
  ProviderConfigView,
  ProviderSettingItem,
} from "@/lib/types";
import { useProviderConfigView, useRemoveProviderConfigEntry } from "@/features/agents/queries";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "ghost";

function toneVariant(tone: ProviderConfigTone): BadgeVariant {
  switch (tone) {
    case "ok":
      return "secondary";
    case "warning":
      return "outline";
    case "danger":
      return "destructive";
    case "muted":
      return "ghost";
  }
}

function renderValue(value: unknown, t: ReturnType<typeof useI18n>["t"]): string {
  if (value === null || value === undefined) return "-";
  if (typeof value === "boolean") return value ? t("yes") : t("no");
  return String(value);
}

function FactRow({ row }: { row: ProviderConfigRow }) {
  const { t } = useI18n();
  return (
    <div className="grid gap-3 border-b py-3 md:grid-cols-[minmax(160px,0.42fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-1">
        <strong className="text-sm font-medium">{row.label}</strong>
        {row.hint ? <span className="text-muted-foreground text-xs">{row.hint}</span> : null}
      </div>
      <div className="text-muted-foreground flex min-w-0 items-center gap-2 break-words font-mono text-xs">
        {row.tone ? (
          <Badge variant={toneVariant(row.tone)}>{renderValue(row.value, t)}</Badge>
        ) : (
          <span>{renderValue(row.value, t)}</span>
        )}
      </div>
    </div>
  );
}

function IssueRow({ issue }: { issue: ProviderConfigIssue }) {
  return (
    <li className="flex items-start gap-2">
      <Badge variant={toneVariant(issue.tone)} className="mt-0.5 capitalize">
        {issue.tone}
      </Badge>
      <span className="text-muted-foreground text-xs">{issue.message}</span>
    </li>
  );
}

function entryId(entry: ProviderConfigEntry) {
  return entry.entry_id || entry.entryId || entry.id;
}

function entryName(entry: ProviderConfigEntry) {
  return entry.name || entry.label || entry.title;
}

function entryFingerprint(entry: ProviderConfigEntry) {
  return entry.expected_fingerprint || entry.expectedFingerprint || entry.fingerprint;
}

function entryRemovable(entry: ProviderConfigEntry) {
  return entry.removable === true || entry.can_remove === true || entry.canRemove === true;
}

function RemoveMcpEntryAction({
  provider,
  view,
  entry,
}: {
  provider: AgentManagementEntry;
  view: ProviderConfigView;
  entry: ProviderConfigEntry;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const mutation = useRemoveProviderConfigEntry();
  const name = entryName(entry) || t("mcpServer");
  const scope = entry.scope || t("unknownScope");
  const source = entry.source || entry.source_path || entry.sourcePath || t("unknownSource");
  const id = entryId(entry);
  const fingerprint = entryFingerprint(entry);
  const viewId = view.view_id || view.viewId;
  const failureMessage = mutation.error instanceof Error ? mutation.error.message : null;
  const conflict = mutation.error &&
    ("status" in mutation.error && mutation.error.status === 409 ||
      /conflict|fingerprint|changed/i.test(failureMessage || ""));

  function close(nextOpen: boolean) {
    if (!nextOpen && !mutation.isPending) {
      mutation.reset();
      setOpen(false);
    }
  }

  function remove() {
    if (!id || !viewId || !fingerprint || mutation.isPending) return;
    mutation.mutate(
      { provider: provider.provider_id, viewId: viewId || "", entryId: id, expectedFingerprint: fingerprint },
      {
        onSuccess: (result) => {
          const status = result.status || result.outcome || result.result;
          toast.success(status === "already_absent" ? t("mcpAlreadyRemoved") : t("mcpRemoved"), {
            description: `${provider.name} · ${name}`,
          });
          setOpen(false);
        },
      },
    );
  }

  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        aria-label={`${t("removeMcpConfiguration")}: ${name}`}
        disabled={!id || !viewId || !fingerprint}
        onClick={() => setOpen(true)}
      >
        <Trash2 data-icon="inline-start" />
        {t("remove")}
      </Button>
      <AlertDialog open={open} onOpenChange={close}>
        <AlertDialogContent data-remove-mcp-dialog>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("removeMcpConfigurationQuestion")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("removeMcpConfigurationDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="rounded-md border bg-muted/30 p-3 text-sm">
            <div><strong>{t("provider")}:</strong> {provider.name || provider.provider_id}</div>
            <div><strong>MCP:</strong> {name}</div>
            <div><strong>{t("scope")}:</strong> {scope}</div>
            <div className="break-all"><strong>{t("source")}:</strong> {source}</div>
          </div>
          {failureMessage ? (
            <p className="text-sm text-destructive">
              {conflict
                ? t("mcpRemovalConflict")
                : `${t("mcpRemovalFailed")} ${failureMessage}`}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={mutation.isPending}>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={!id || !viewId || !fingerprint || mutation.isPending}
              onClick={(event) => {
                event.preventDefault();
                remove();
              }}
            >
              {mutation.isPending ? <LoaderCircle className="animate-spin" data-icon="inline-start" /> : null}
              {mutation.error ? t("retry") : t("remove")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function ConfigViewContent({ provider, view, canRemoveMcp }: { provider: AgentManagementEntry; view: ProviderConfigView; canRemoveMcp: boolean }) {
  const { t } = useI18n();
  const sources = view.sources?.filter((source) => source.path).map((source) =>
    source.exists ? source.path : `${source.path} (${t("missing")})`,
  );
  return (
    <div className="flex flex-col gap-4">
      {sources && sources.length > 0 ? (
        <div className="text-muted-foreground text-xs">{t("readFrom")} {sources.join(", ")}</div>
      ) : null}
      {view.sections?.map((section) => (
        <div key={section.label} className="flex flex-col">
          <div className="flex items-center justify-between gap-3 pb-1">
            <strong className="text-foreground text-sm font-medium">{section.label}</strong>
            {canRemoveMcp ? (() => {
              const entry = section.entry || view.entries?.find((candidate) => entryName(candidate) === section.label) ||
                (view.entries?.length === 1 ? view.entries[0] : undefined);
              return entry && entryRemovable(entry) ? (
                <RemoveMcpEntryAction provider={provider} view={view} entry={entry} />
              ) : null;
            })() : null}
          </div>
          {section.rows.map((row) => (
            <FactRow key={row.label} row={row} />
          ))}
        </div>
      ))}
      {view.issues && view.issues.length > 0 ? (
        <ul className="flex flex-col gap-2">
          {view.issues.map((issue, index) => (
            <IssueRow key={index} issue={issue} />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function ConfigViewPanel({
  provider,
  view,
  compact,
}: {
  provider: AgentManagementEntry;
  view: ProviderSettingItem;
  compact: boolean;
}) {
  const { t } = useI18n();
  const result = useProviderConfigView(provider.provider_id, view.id, true);

  return (
    <div className="flex flex-col gap-3">
      {compact ? null : (
        <div className="flex min-w-0 flex-col gap-0.5">
          <strong className="text-sm font-medium">{view.title}</strong>
          <span className="text-muted-foreground text-xs">{view.description}</span>
        </div>
      )}
      {result.isLoading ? <Skeleton className="h-16 w-full" /> : null}
      {result.error ? (
        <PageError title={t("configViewLoadFailed", { title: view.title })} message={result.error.message} />
      ) : null}
      {result.data ? (
        <ConfigViewContent
          provider={provider}
          view={result.data}
          canRemoveMcp={
            view.id === "view_mcp" &&
            (provider.capabilities?.mcp_management?.remove === true ||
              result.data.entries?.some(entryRemovable) === true)
          }
        />
      ) : null}
    </div>
  );
}

/**
 * Renders `View`-kind provider settings for a dedicated agent-detail tab.
 * Declarations ride the existing agent-detail payload; content loads when the
 * tab mounts.
 */
export function ConfigViewsBlock({
  provider,
  viewFilter,
}: {
  provider: AgentManagementEntry;
  viewFilter?: (view: ProviderSettingItem) => boolean;
}) {
  const views = (provider.settings || []).filter(
    (setting) => setting.kind === "view" && (viewFilter ? viewFilter(setting) : true),
  );
  if (views.length === 0) return null;
  const compact = views.length === 1;
  return (
    <section className="flex flex-col gap-6" data-config-views>
      {views.map((view) => (
        <ConfigViewPanel
          key={view.id}
          provider={provider}
          view={view}
          compact={compact}
        />
      ))}
    </section>
  );
}
