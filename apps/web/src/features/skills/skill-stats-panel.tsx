import { Link } from "react-router-dom";
import { useMemo, useState, useEffect, type ReactNode } from "react";
import { RefreshCwIcon } from "lucide-react";
import { toast } from "sonner";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Progress } from "@/components/ui/progress";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAnalyzeSkills, useCurrentSkillAnalysis, useSkillAnalysisOperation, useSkillInvocations, useSkillStats } from "@/features/skills/queries";
import {
  useSkillStatsFilters,
} from "@/features/skills/skill-stats-filters";
import { workspaceName } from "@/components/shared/workspace-name";
import type {
  SkillRanking,
  SkillStatsBreakdownItem,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { useI18n } from "@/lib/i18n-context";
import { useUiStore } from "@/stores/ui-store";

function formatTime(value?: number | null) {
  return value ? new Date(value).toLocaleString() : "—";
}

type WorkspaceBreakdownRow = SkillStatsBreakdownItem & {
  paths: string[];
};

function aggregateWorkspaceBreakdown(
  items: SkillStatsBreakdownItem[],
): WorkspaceBreakdownRow[] {
  const grouped = new Map<
    string,
    { invocations: number; sessions: number; paths: string[] }
  >();

  for (const item of items) {
    const label = workspaceName(item.key, item.key);
    const bucket = grouped.get(label) ?? {
      invocations: 0,
      sessions: 0,
      paths: [],
    };
    bucket.invocations += item.invocations;
    bucket.sessions += item.sessions;
    if (!bucket.paths.includes(item.key)) bucket.paths.push(item.key);
    grouped.set(label, bucket);
  }

  return Array.from(grouped.entries())
    .map(([label, stats]) => ({
      key: label,
      invocations: stats.invocations,
      sessions: stats.sessions,
      paths: stats.paths,
    }))
    .sort(
      (left, right) =>
        right.invocations - left.invocations ||
        left.key.localeCompare(right.key),
    );
}

function WorkspaceBreakdownTick({
  x = 0,
  y = 0,
  index = 0,
  rows,
  onQuickSwitch,
}: {
  x?: string | number;
  y?: string | number;
  index?: number;
  rows: WorkspaceBreakdownRow[];
  onQuickSwitch: (paths: string[]) => void;
}) {
  const row = rows[index];
  const name = row?.key ?? "";
  const title = row?.paths.join("\n") ?? name;
  return (
    <foreignObject x={Number(x) - 96} y={Number(y) - 10} width={94} height={20}>
      <button
        type="button"
        title={title}
        className="block w-full cursor-pointer truncate text-left text-[11px] leading-5 underline-offset-2 transition-colors hover:text-primary hover:underline"
        onClick={(event) => {
          event.stopPropagation();
          if (row?.paths.length) onQuickSwitch(row.paths);
        }}
      >
        {name}
      </button>
    </foreignObject>
  );
}

function BreakdownBarChart({
  kind,
  providerData,
  workspaceRows,
  onQuickSwitchWorkspace,
}: {
  kind: "providers" | "workspaces";
  providerData: SkillStatsBreakdownItem[];
  workspaceRows: WorkspaceBreakdownRow[];
  onQuickSwitchWorkspace: (paths: string[]) => void;
}) {
  const chartData = kind === "workspaces" ? workspaceRows : providerData;
  const chartHeight = Math.max(176, chartData.length * 32);

  return (
    <div style={{ height: chartHeight }}>
      <ResponsiveContainer width="100%" height="100%">
        <BarChart
          data={chartData}
          layout="vertical"
          margin={{ left: 18, right: 8 }}
        >
          <CartesianGrid strokeDasharray="3 3" horizontal={false} />
          <XAxis type="number" allowDecimals={false} />
          <YAxis
            type="category"
            dataKey="key"
            width={kind === "workspaces" ? 100 : 90}
            interval={0}
            tick={
              kind === "workspaces"
                ? (props) => (
                    <WorkspaceBreakdownTick
                      {...props}
                      rows={workspaceRows}
                      onQuickSwitch={onQuickSwitchWorkspace}
                    />
                  )
                : { fontSize: 11 }
            }
          />
          <Tooltip
            labelFormatter={(value, items) => {
              if (kind !== "workspaces") return String(value);
              const row = items?.[0]?.payload as WorkspaceBreakdownRow | undefined;
              return row?.paths.join(" · ") ?? String(value);
            }}
          />
          <Bar
            dataKey="invocations"
            fill="var(--primary)"
            radius={[0, 3, 3, 0]}
          />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

function StatsPanelSection({
  borderless,
  title,
  header,
  children,
  className,
}: {
  borderless?: boolean;
  title?: ReactNode;
  header?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  if (borderless) {
    return (
      <section className={cn("py-4 first:pt-0", className)}>
        {header ? <div className="mb-3 grid gap-3">{header}</div> : null}
        {!header && title ? (
          <h3 className="mb-3 text-base font-semibold">{title}</h3>
        ) : null}
        {children}
      </section>
    );
  }
  return (
    <Card className={className}>
      {header ? (
        <CardHeader className="gap-3 pb-2">{header}</CardHeader>
      ) : title ? (
        <CardHeader className="pb-2">
          <CardTitle className="text-base">{title}</CardTitle>
        </CardHeader>
      ) : null}
      <CardContent>{children}</CardContent>
    </Card>
  );
}

export function SkillStatsAnalyzeButton() {
  const { t } = useI18n();
  const analyzeMutation = useAnalyzeSkills();
  const currentAnalysis = useCurrentSkillAnalysis();
  const busy =
    analyzeMutation.isPending ||
    currentAnalysis.data?.status === "queued" ||
    currentAnalysis.data?.status === "running";

  return (
    <Button
      variant="outline"
      size="sm"
      disabled={busy}
      onClick={() =>
        analyzeMutation.mutate("incremental", {
          onSuccess: () => toast.success(t("skillsAnalysisRefreshed")),
          onError: (error) =>
            toast.error(t("skillsAnalysisRefreshFailed"), {
              description: error.message,
            }),
        })
      }
    >
      {busy ? <Spinner /> : <RefreshCwIcon />}
      {busy ? t("skillsAnalysisInProgress") : t("skillsRefreshUsageAnalysis")}
    </Button>
  );
}

export function SkillStatsAnalysisProgress({ className }: { className?: string }) {
  const currentAnalysis = useCurrentSkillAnalysis();
  const analysis = currentAnalysis.data;
  if (!analysis || (analysis.status !== "queued" && analysis.status !== "running")) {
    return null;
  }

  return (
    <div className={`flex min-w-0 items-center gap-3 text-xs text-muted-foreground ${className ?? ""}`}>
      <span className="shrink-0">
        {analysis.processed_sources}/{analysis.total_sources || "—"} · {analysis.percentage}%
      </span>
      <Progress className="min-w-0 flex-1" value={analysis.percentage} />
    </div>
  );
}

export function SkillStatsPanel({
  provider,
  section = "all",
}: {
  provider?: string;
  section?: "all" | "summary" | "ranking";
}) {
  const { t } = useI18n();
  const { params } = useSkillStatsFilters(provider);
  const stats = useSkillStats(params);
  const currentAnalysis = useCurrentSkillAnalysis();
  const analysisOperation = useSkillAnalysisOperation(
    currentAnalysis.data?.operation_id || null,
  );
  useEffect(() => {
    if (analysisOperation.data?.status === "completed") {
      void stats.summary.refetch();
      void stats.daily.refetch();
      void stats.breakdown.refetch();
      void stats.ranking.refetch();
    }
  }, [analysisOperation.data?.status, stats.breakdown, stats.daily, stats.ranking, stats.summary]);
  const openWorkspaceQuickSwitch = useUiStore(
    (state) => state.openWorkspaceQuickSwitch,
  );
  const [evidenceTarget, setEvidenceTarget] = useState<SkillRanking | null>(
    null,
  );
  const [evidencePage, setEvidencePage] = useState(1);
  const evidenceInvocations = useSkillInvocations(evidenceTarget?.skill_id ?? null, {
    ...params,
    page: evidencePage,
    pageSize: 10,
  });
  const summary = stats.summary.data;
  const showSummary = section === "all" || section === "summary";
  const showRanking = section === "all" || section === "ranking";
  const summaryBorderless = section === "summary";
  const workspaceBreakdown = useMemo(
    () => aggregateWorkspaceBreakdown(stats.breakdown.data?.workspaces ?? []),
    [stats.breakdown.data?.workspaces],
  );
  const providerBreakdown = stats.breakdown.data?.providers ?? [];

  const usageHeader = (
    <h3 className="text-base font-semibold">{t("skillsUsageStats")}</h3>
  );

  const usageMetrics = (
    <div className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
      <div>
        <strong className="block text-xl">{summary?.invocations ?? "—"}</strong>
        {t("skillsInvocationLabel")}
      </div>
      <div>
        <strong className="block text-xl">
          {summary?.active_skills ?? "—"}
        </strong>
        {t("skillsActiveSkills")}
      </div>
      <div>
        <strong className="block text-xl">
          {summary?.active_sessions ?? "—"}
        </strong>
        {t("skillsSessionLabel")}
      </div>
      <div>
        <strong className="block text-xl">{summary?.active_days ?? "—"}</strong>
        {t("skillsActiveDays")}
      </div>
      <div className="col-span-2">
        <strong className="block text-sm">
          {formatTime(summary?.last_invoked_at_ms)}
        </strong>
        {t("skillsLastInvocation")}
      </div>
    </div>
  );

  const summarySections = showSummary ? (
    <>
      <StatsPanelSection borderless={summaryBorderless} header={usageHeader}>
        <div className="grid gap-3">
          {usageMetrics}
        </div>
      </StatsPanelSection>
      <StatsPanelSection borderless={summaryBorderless} title={t("skillsDailyInvocations")}>
        <div className="h-44">
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart
              data={stats.daily.data ?? []}
              margin={{ left: -24, right: 8 }}
            >
              <CartesianGrid strokeDasharray="3 3" vertical={false} />
              <XAxis dataKey="date" tick={{ fontSize: 11 }} minTickGap={24} />
              <YAxis allowDecimals={false} tick={{ fontSize: 11 }} />
              <Tooltip />
              <Area
                type="monotone"
                dataKey="invocations"
                stroke="var(--primary)"
                fill="var(--primary)"
                fillOpacity={0.18}
              />
            </AreaChart>
          </ResponsiveContainer>
        </div>
      </StatsPanelSection>
      {(["providers", "workspaces"] as const).map((kind) => (
        <StatsPanelSection
          key={kind}
          borderless={summaryBorderless}
          title={kind === "providers" ? t("skillsProviderDistribution") : t("skillsProjectDistribution")}
        >
          <BreakdownBarChart
            kind={kind}
            providerData={providerBreakdown}
            workspaceRows={workspaceBreakdown}
            onQuickSwitchWorkspace={openWorkspaceQuickSwitch}
          />
        </StatsPanelSection>
      ))}
    </>
  ) : null;

  return (
    <section
      className={cn(
        "min-w-0",
        section === "all" && "grid shrink-0 gap-3 xl:grid-cols-2",
        section === "summary" && "shrink-0",
        section === "ranking" && "shrink-0",
      )}
    >
      {showSummary && summaryBorderless ? (
        <div className="flex flex-col divide-y divide-border">{summarySections}</div>
      ) : (
        summarySections
      )}

      {showRanking ? (
      <>
      <div className={cn("min-w-0", section === "all" && "xl:col-span-2")}>
          <Table className="table-fixed">
            <TableHeader>
              <TableRow>
                <TableHead className="w-2/5">{t("skillsColumnSkill")}</TableHead>
                <TableHead className="w-[18%]">{t("skillsInvocationLabel")}</TableHead>
                <TableHead className="w-[18%]">{t("skillsSessionLabel")}</TableHead>
                <TableHead className="w-[27%]">{t("skillsRecent")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {(stats.ranking.data ?? []).slice(0, 8).map((item) => (
                <TableRow
                  key={item.skill_id}
                  className="cursor-pointer"
                  data-state={
                    evidenceTarget?.skill_id === item.skill_id
                      ? "selected"
                      : undefined
                  }
                  onClick={() => {
                    setEvidencePage(1);
                    setEvidenceTarget(item);
                  }}
                >
                  <TableCell className="truncate">{item.name}</TableCell>
                  <TableCell>{item.invocations}</TableCell>
                  <TableCell>{item.sessions}</TableCell>
                  <TableCell className="truncate text-xs">
                    {formatTime(item.last_invoked_at_ms)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {!stats.ranking.data?.length ? (
            <p className="text-muted-foreground mt-3 text-sm">{t("skillsNoRanking")}</p>
          ) : (
            <p className="text-muted-foreground mt-3 text-xs">
              {t("skillsClickRowForEvidence")}
            </p>
          )}
      </div>
      <Sheet
        open={Boolean(evidenceTarget)}
        onOpenChange={(open) => {
          if (!open) {
            setEvidenceTarget(null);
            setEvidencePage(1);
          }
        }}
      >
        <SheetContent className="overflow-y-auto sm:max-w-md">
          <SheetHeader>
            <SheetTitle>{t("skillsInvocationEvidence", { skill: evidenceTarget?.name ?? "Skill" })}</SheetTitle>
            <SheetDescription>
              {t("skillsInvocationEvidenceDescription")}
            </SheetDescription>
          </SheetHeader>
          <div className="space-y-2 px-4 pb-4">
            {(evidenceInvocations.data?.items ?? []).map((item) => (
              <div key={item.id} className="rounded-md border p-2 text-xs">
                <div className="flex flex-wrap items-center gap-2">
                  <time>{formatTime(item.invoked_at_ms)}</time>
                  <Badge variant="outline">{item.provider_id}</Badge>
                  <Badge variant="outline">{item.detection_kind}</Badge>
                  <Badge
                    variant={
                      item.confidence === "low" ? "secondary" : "default"
                    }
                  >
                    {item.confidence}
                  </Badge>
                  <Link
                    className="text-primary underline"
                    to={`/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`}
                  >
                    {t("skillsOpenSession", { session: "" })}
                  </Link>
                </div>
                <p className="text-muted-foreground mt-1">
                  {t("skillsProject", { project: item.workspace_dir || t("skillsUnspecified") })}
                </p>
                <p className="text-muted-foreground mt-1 line-clamp-3">
                  {item.evidence_text || item.evidence_path || t("skillsNoEvidenceSummary")}
                </p>
              </div>
            ))}
            {evidenceTarget &&
            evidenceInvocations.data &&
            evidenceInvocations.data.items.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                {t("skillsNoInvocationEvidence")}
              </p>
            ) : null}
            {evidenceInvocations.data &&
            evidenceInvocations.data.total > evidenceInvocations.data.page_size ? (
              <div className="flex items-center justify-end gap-2 pt-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={evidencePage <= 1}
                  onClick={() => setEvidencePage((value) => value - 1)}
                >
                  {t("skillsPreviousPage")}
                </Button>
                <span className="text-xs">{t("skillsPageNumber", { page: evidencePage })}</span>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={
                    evidencePage * evidenceInvocations.data.page_size >=
                    evidenceInvocations.data.total
                  }
                  onClick={() => setEvidencePage((value) => value + 1)}
                >
                  {t("skillsNextPage")}
                </Button>
              </div>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>
      </>
      ) : null}
    </section>
  );
}
