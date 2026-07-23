import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  useSkillGraph,
  useSkillInvocations,
  useSkillStats,
} from "@/features/skills/queries";
import { useI18n } from "@/lib/i18n-context";

const GRAPH_WEEKS = 52;

const colors = [
  "bg-muted",
  "bg-emerald-200 dark:bg-emerald-950",
  "bg-emerald-400 dark:bg-emerald-800",
  "bg-emerald-600",
  "bg-emerald-800 dark:bg-emerald-400",
];

function localDate(date: Date) {
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
    .toISOString()
    .slice(0, 10);
}

export function SkillGraphPanel({
  skillId,
  provider,
  embedded = false,
}: {
  skillId: string | null;
  provider?: string;
  embedded?: boolean;
}) {
  const { t } = useI18n();
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const params = useMemo(() => {
    const to = new Date();
    const from = new Date(to);
    from.setDate(from.getDate() - GRAPH_WEEKS * 7 + 1);
    return {
      from: localDate(from),
      to: localDate(to),
      skillId: skillId ?? undefined,
      provider,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    };
  }, [provider, skillId]);
  const graph = useSkillGraph(params);
  const dayParams = {
    from: selectedDate ?? undefined,
    to: selectedDate ?? undefined,
    skillId: skillId ?? undefined,
    provider,
    pageSize: 50,
  };
  const dayStats = useSkillStats(dayParams);
  const invocations = useSkillInvocations(skillId, dayParams);
  const selected = graph.data?.days.find((day) => day.date === selectedDate);
  const trend = useMemo(
    () =>
      graph.data?.days.map((day, index, days) => ({
        date: day.date,
        rolling: days
          .slice(Math.max(0, index - 6), index + 1)
          .reduce((sum, item) => sum + item.invocations, 0),
      })) ?? [],
    [graph.data?.days],
  );
  const content = (
    <>
      {!embedded ? (
        <SectionHeading title={t("skillsActivityHeatmap")} className="border-0 pb-0" />
      ) : null}
      {graph.isError ? (
        <p className="text-sm text-destructive">{graph.error.message}</p>
      ) : (
        <div className="overflow-x-auto pb-2">
          <div
            className="grid w-max grid-flow-col grid-rows-7 gap-1"
            role="grid"
            aria-label={t("skillsDailyInvocations")}
          >
            {graph.data?.days.map((day) => (
              <button
                key={day.date}
                className={`size-3 rounded-[2px] ${colors[day.level]}`}
                title={t("skillsDaySummary", { date: day.date, invocations: day.invocations, sessions: day.sessions, skills: day.active_skills })}
                aria-label={t("skillsDaySummary", { date: day.date, invocations: day.invocations, sessions: day.sessions, skills: day.active_skills })}
                onClick={() => setSelectedDate(day.date)}
              />
            ))}
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-4 text-sm text-muted-foreground">
        <span>{t("skillsTotalInvocations")} {graph.data?.total_invocations ?? "—"}</span>
        <span>{t("skillsPeak")} {graph.data?.max_count ?? "—"}</span>
      </div>
      <div className="h-36" aria-label={t("skillsRollingTrend")}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={trend} margin={{ left: -24, right: 8 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} />
            <XAxis dataKey="date" tick={{ fontSize: 11 }} minTickGap={24} />
            <YAxis allowDecimals={false} tick={{ fontSize: 11 }} />
            <Tooltip />
            <Area
              type="monotone"
              dataKey="rolling"
              name={t("skillsRollingTrend")}
              stroke="var(--primary)"
              fill="var(--primary)"
              fillOpacity={0.18}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <Sheet open={Boolean(selectedDate)} onOpenChange={(open) => !open && setSelectedDate(null)}>
        <SheetContent className="overflow-y-auto">
          <SheetHeader>
            <SheetTitle>{selectedDate} {t("skillsInvocations")}</SheetTitle>
            <SheetDescription>
              {selected
                ? t("skillsDaySummary", { date: selected.date, invocations: selected.invocations, sessions: selected.sessions, skills: selected.active_skills })
                : t("skillsLoadingDayInvocations")}
            </SheetDescription>
          </SheetHeader>
          <div className="space-y-4 px-4 pb-4">
            <section>
              <h3 className="mb-2 font-medium">{t("skillsDailyRanking")}</h3>
              {(dayStats.ranking.data ?? []).map((item) => (
                <div key={item.skill_id} className="flex justify-between border-b py-2 text-sm">
                  <span>{item.name}</span>
                  <span>{t("skillsInvocationCount", { count: item.invocations })}</span>
                </div>
              ))}
            </section>
            <section>
              <h3 className="mb-2 font-medium">{t("skillsInvocationDetails")}</h3>
              {!skillId ? (
                <p className="text-sm text-muted-foreground">{t("skillsSelectForEvidence")}</p>
              ) : (
                (invocations.data?.items ?? []).map((item) => (
                  <Link
                    key={item.id}
                    className="block border-b py-2 text-sm text-primary underline"
                    to={`/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`}
                  >
                    {item.provider_id} · {item.detection_kind}
                  </Link>
                ))
              )}
            </section>
          </div>
        </SheetContent>
      </Sheet>
    </>
  );

  if (embedded) {
    return <div className="space-y-3">{content}</div>;
  }

  return <PanelCard className="space-y-3 p-4">{content}</PanelCard>;
}
