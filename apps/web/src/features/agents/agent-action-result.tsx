import { Link } from "react-router-dom";
import { MetricGrid, MetricTile } from "@/components/shared/metric-grid";
import { PathText } from "@/components/shared/path-text";
import { useI18n } from "@/lib/i18n-context";
import { workspaceName } from "@/components/shared/workspace-name";
import { Badge } from "@/components/ui/badge";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import type { CodexWorkspaceRepairItem, CodexWorkspaceRepairReport, ProviderSettingOutput } from "@/lib/types";

function isCodexWorkspaceRepairReport(value: unknown): value is CodexWorkspaceRepairReport {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return typeof record.workspace_dir === "string" && Array.isArray(record.touched_sessions);
}

function sessionLabel(item: CodexWorkspaceRepairItem) {
  const title = item.title?.trim();
  if (!title) return item.session_id;
  const firstLine = title.split(/\r?\n/, 1)[0]?.trim();
  return firstLine || item.session_id;
}

function sessionChanges(item: CodexWorkspaceRepairItem) {
  const changes: string[] = [];
  if (item.updated_model_provider) changes.push("provider");
  if (item.added_to_index) changes.push("indexed");
  if (item.updated_index_title) changes.push("title");
  return changes;
}

function CodexWorkspaceRepairResult({
  providerId,
  report,
}: {
  providerId: string;
  report: CodexWorkspaceRepairReport;
}) {
  const { t } = useI18n();
  const touched = report.touched_sessions ?? [];

  return (
    <div className="flex flex-col gap-4" data-codex-workspace-repair-result>
      <MetricGrid columns="auto">
        <MetricTile label={t("repaired")} value={report.repaired_session_count} hint={t("sessionsSynced")} variant="compact" />
        <MetricTile label={t("hidden")} value={report.hidden_session_count} hint={t("beforeRepair")} variant="compact" />
        <MetricTile label={t("reindexed")} value={report.reindexed_session_count} variant="compact" />
        <MetricTile label={t("retitled")} value={report.retitled_session_count ?? 0} variant="compact" />
      </MetricGrid>

      <div className="grid gap-2 rounded-md border p-3 text-sm">
        <div className="grid gap-1 md:grid-cols-[minmax(140px,auto)_minmax(0,1fr)] md:items-start">
          <span className="text-muted-foreground font-mono text-xs uppercase">{t("workspace")}</span>
          <div className="min-w-0">
            <strong className="block truncate">{workspaceName(report.workspace_dir)}</strong>
            <PathText value={report.workspace_dir} wrap="all" />
          </div>
        </div>
        <div className="grid gap-1 md:grid-cols-[minmax(140px,auto)_minmax(0,1fr)] md:items-center">
          <span className="text-muted-foreground font-mono text-xs uppercase">{t("provider")}</span>
          <span>{report.current_model_provider}</span>
        </div>
        <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 font-mono text-xs">
          <span>{t("scannedRollouts", { count: report.scanned_rollouts })}</span>
          <span>{t("workspaceSessions", { count: report.workspace_session_count })}</span>
          <span>{t("sqliteRowsUpdated", { count: report.sqlite_rows_updated })}</span>
          {report.pruned_backup_count > 0 ? <span>{t("backupsPruned", { count: report.pruned_backup_count })}</span> : null}
        </div>
        {report.backup_dir ? (
          <div className="grid gap-1 md:grid-cols-[minmax(140px,auto)_minmax(0,1fr)] md:items-start">
            <span className="text-muted-foreground font-mono text-xs uppercase">{t("backup")}</span>
            <PathText value={report.backup_dir} wrap="all" />
          </div>
        ) : null}
      </div>

      <div className="flex flex-col gap-2">
        <strong className="text-sm font-medium">{t("restoredSessions", { count: touched.length })}</strong>
        {touched.length ? (
          <div className="flex flex-col gap-2">
            {touched.map((item) => {
              const href = `/sessions/${encodeURIComponent(providerId)}/${encodeURIComponent(item.session_id)}`;
              const changes = sessionChanges(item);
              return (
                <div key={item.session_id} className="grid gap-2 border-b py-3 last:border-b-0">
                  <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
                    <Link to={href} className="min-w-0 text-sm font-medium hover:underline">
                      <span className="line-clamp-2">{sessionLabel(item)}</span>
                    </Link>
                    <ButtonLink href={href} />
                  </div>
                  <div className="text-muted-foreground flex flex-wrap gap-2 font-mono text-xs">
                    <Badge variant="outline">{item.session_id}</Badge>
                    {item.previous_model_provider && item.previous_model_provider !== item.current_model_provider ? (
                      <Badge variant="outline">
                        {item.previous_model_provider} → {item.current_model_provider}
                      </Badge>
                    ) : (
                      <Badge variant="outline">{item.current_model_provider}</Badge>
                    )}
                    {changes.map((change) => (
                      <Badge key={change} variant="secondary">{change}</Badge>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>{t("noSessionsNeededRepair")}</EmptyTitle>
              <EmptyDescription>{t("allSessionsVisible")}</EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      </div>

      {report.skipped_rollout_files?.length ? (
        <div className="flex flex-col gap-2">
          <strong className="text-sm font-medium">{t("skippedRollouts", { count: report.skipped_rollout_files.length })}</strong>
          <div className="flex flex-col gap-1">
            {report.skipped_rollout_files.map((path) => (
              <PathText key={path} value={path} wrap="all" />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ButtonLink({ href }: { href: string }) {
  return (
    <Link to={href} className="text-muted-foreground shrink-0 text-xs hover:underline">
      View
    </Link>
  );
}

export function AgentActionResultPanel({
  providerId,
  result,
}: {
  providerId: string;
  result: unknown;
}) {
  const { t } = useI18n();
  const output = result as ProviderSettingOutput;
  if (output?.type === "codex_workspace_repair" && isCodexWorkspaceRepairReport(output.data)) {
    return <CodexWorkspaceRepairResult providerId={providerId} report={output.data} />;
  }

  return (
    <Empty>
      <EmptyHeader>
        <EmptyTitle>{t("actionCompleted")}</EmptyTitle>
        <EmptyDescription>{t("noStructuredResultRenderer")}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}
