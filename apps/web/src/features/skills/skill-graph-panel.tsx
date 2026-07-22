import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import { useSkillGraph } from "@/features/skills/queries";

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
  const [weeks, setWeeks] = useState(52);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const params = useMemo(() => {
    const to = new Date();
    const from = new Date(to);
    from.setDate(from.getDate() - weeks * 7 + 1);
    return {
      from: localDate(from),
      to: localDate(to),
      skillId: skillId ?? undefined,
      provider,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    };
  }, [provider, skillId, weeks]);
  const graph = useSkillGraph(params);
  const selected = graph.data?.days.find((day) => day.date === selectedDate);
  return (
    <PanelCard className="space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SectionHeading title="Skill 活跃热力图" className="border-0 pb-0" />
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
        {selected && (
          <span>
            {selected.date}：{selected.invocations} 调用 / {selected.sessions}{" "}
            会话 / {selected.active_skills} Skill ·{" "}
            <Link
              className="text-primary underline"
              to={`/sessions?date=${selected.date}`}
            >
              查看会话
            </Link>
          </span>
        )}
      </div>
    </PanelCard>
  );
}
