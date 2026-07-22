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
import { useUiStore } from "@/stores/ui-store";

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
}: {
  skillId: string | null;
  provider?: string;
}) {
  const currentWorkspace = useUiStore((state) => state.selectedWorkspace) ?? undefined;
  const [weeks, setWeeks] = useState(52);
  const [projectOnly, setProjectOnly] = useState(false);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const workspace = projectOnly ? currentWorkspace : undefined;
  const params = useMemo(() => {
    const to = new Date();
    const from = new Date(to);
    from.setDate(from.getDate() - weeks * 7 + 1);
    return {
      from: localDate(from),
      to: localDate(to),
      skillId: skillId ?? undefined,
      provider,
      workspace,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    };
  }, [provider, skillId, weeks, workspace]);
  const graph = useSkillGraph(params);
  const dayParams = {
    from: selectedDate ?? undefined,
    to: selectedDate ?? undefined,
    skillId: skillId ?? undefined,
    provider,
    workspace,
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
  return (
    <PanelCard className="space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SectionHeading title="Skill 活跃热力图" className="border-0 pb-0" />
        <div className="flex gap-2">
          <select
            aria-label="项目范围"
            className="h-8 max-w-48 rounded-md border bg-background px-2 text-sm"
            value={projectOnly ? "current" : "all"}
            onChange={(event) => setProjectOnly(event.target.value === "current")}
          >
            <option value="all">全部项目</option>
            <option value="current" disabled={!currentWorkspace}>
              当前项目{currentWorkspace ? `：${currentWorkspace}` : ""}
            </option>
          </select>
          <select
            aria-label="热力图范围"
            className="h-8 rounded-md border bg-background px-2 text-sm"
            value={weeks}
            onChange={(event) => setWeeks(Number(event.target.value))}
          >
            <option value={13}>13 周</option>
            <option value={26}>26 周</option>
            <option value={52}>52 周</option>
          </select>
        </div>
      </div>
      {graph.isError ? (
        <p className="text-sm text-destructive">{graph.error.message}</p>
      ) : (
        <div className="overflow-x-auto pb-2">
          <div
            className="grid w-max grid-flow-col grid-rows-7 gap-1"
            role="grid"
            aria-label="每日 Skill 调用"
          >
            {graph.data?.days.map((day) => (
              <button
                key={day.date}
                className={`size-3 rounded-[2px] ${colors[day.level]}`}
                title={`${day.date}: ${day.invocations} 次调用，${day.sessions} 个会话，${day.active_skills} 个 Skill`}
                aria-label={`${day.date}，${day.invocations} 次调用，${day.sessions} 个会话，${day.active_skills} 个活跃 Skill`}
                onClick={() => setSelectedDate(day.date)}
              />
            ))}
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-4 text-sm text-muted-foreground">
        <span>总调用 {graph.data?.total_invocations ?? "—"}</span>
        <span>峰值 {graph.data?.max_count ?? "—"}</span>
      </div>
      <div className="h-36" aria-label="7 日滚动调用趋势">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={trend} margin={{ left: -24, right: 8 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} />
            <XAxis dataKey="date" tick={{ fontSize: 11 }} minTickGap={24} />
            <YAxis allowDecimals={false} tick={{ fontSize: 11 }} />
            <Tooltip />
            <Area
              type="monotone"
              dataKey="rolling"
              name="7 日调用"
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
            <SheetTitle>{selectedDate} Skill 调用</SheetTitle>
            <SheetDescription>
              {selected
                ? `${selected.invocations} 次调用，${selected.sessions} 个会话，${selected.active_skills} 个活跃 Skill`
                : "加载当日调用数据"}
            </SheetDescription>
          </SheetHeader>
          <div className="space-y-4 px-4 pb-4">
            <section>
              <h3 className="mb-2 font-medium">当日排名</h3>
              {(dayStats.ranking.data ?? []).map((item) => (
                <div key={item.skill_id} className="flex justify-between border-b py-2 text-sm">
                  <span>{item.name}</span>
                  <span>{item.invocations} 次</span>
                </div>
              ))}
            </section>
            <section>
              <h3 className="mb-2 font-medium">调用明细</h3>
              {!skillId ? (
                <p className="text-sm text-muted-foreground">选择一个 Skill 后查看调用证据。</p>
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
    </PanelCard>
  );
}
