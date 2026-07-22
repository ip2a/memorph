import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PanelCard } from "@/components/shared/panel-card";
import { SectionHeading } from "@/components/shared/section-heading";
import { useSkillConflicts, useSkillCoverage } from "@/features/skills/queries";

const ranges = ["7d", "30d", "90d", "all"];

export function SkillCoverageConflictsPanel({
  skillId,
}: {
  skillId: string | null;
}) {
  const [range, setRange] = useState("90d");
  const coverage = useSkillCoverage(skillId, range);
  const conflicts = useSkillConflicts(skillId);
  if (!skillId) return null;
  return (
    <div className="grid gap-3 lg:grid-cols-2">
      <PanelCard className="space-y-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SectionHeading title="覆盖率" className="border-0 pb-0" />
          <div className="flex gap-1">
            {ranges.map((item) => (
              <Button
                key={item}
                size="sm"
                variant={range === item ? "default" : "outline"}
                onClick={() => setRange(item)}
              >
                {item}
              </Button>
            ))}
          </div>
        </div>
        {coverage.isLoading ? (
          <p className="text-sm text-muted-foreground">加载中…</p>
        ) : coverage.isError ? (
          <p className="text-sm text-destructive">{coverage.error.message}</p>
        ) : (
          <>
            <p className="text-2xl font-semibold">
              {coverage.data?.percent.toFixed(1)}%{" "}
              <span className="text-sm font-normal text-muted-foreground">
                {coverage.data?.covered}/{coverage.data?.total}
              </span>
            </p>
            <div className="max-h-48 space-y-2 overflow-auto">
              {coverage.data?.targets.map((target) => (
                <div
                  key={`${target.target_kind}:${target.target_key}`}
                  className="flex items-start justify-between gap-2 rounded-md border p-2 text-sm"
                >
                  <span>
                    {target.section_title ??
                      target.target_path ??
                      target.target_key}
                  </span>
                  <Badge variant="outline">
                    {target.observations} · {target.confidence ?? "未覆盖"}
                  </Badge>
                </div>
              ))}
            </div>
          </>
        )}
      </PanelCard>
      <PanelCard className="space-y-3 p-4">
        <SectionHeading title="触发冲突" className="border-0 pb-0" />
        {conflicts.isLoading ? (
          <p className="text-sm text-muted-foreground">加载中…</p>
        ) : conflicts.isError ? (
          <p className="text-sm text-destructive">{conflicts.error.message}</p>
        ) : conflicts.data?.length ? (
          <div className="max-h-64 space-y-2 overflow-auto">
            {conflicts.data.map((item) => (
              <div key={item.id} className="rounded-md border p-3 text-sm">
                <div className="flex items-center gap-2">
                  <Badge
                    variant={
                      item.severity === "error" ? "destructive" : "outline"
                    }
                  >
                    {item.conflict_kind}
                  </Badge>
                  <span>{Math.round(item.similarity * 100)}%</span>
                </div>
                <p className="mt-2 font-medium">
                  {item.left_name} ↔ {item.right_name}
                </p>
                <p className="text-muted-foreground">{item.evidence}</p>
                <p className="text-muted-foreground">
                  建议：{item.recommendation}
                </p>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">未发现冲突。</p>
        )}
      </PanelCard>
    </div>
  );
}
