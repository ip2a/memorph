import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  useSkillContext,
  useSkillContextSummary,
  useSkillHealth,
  useSkillHealthSummary,
} from "@/features/skills/queries";

const BASELINES = [32_000, 128_000, 200_000] as const;

export function SkillContextHealthPanel({
  skillId,
  provider,
}: {
  skillId: string | null;
  provider?: string;
}) {
  const [baseline, setBaseline] = useState<number>();
  const contextSummary = useSkillContextSummary(provider, baseline);
  const context = useSkillContext(skillId, baseline);
  const healthSummary = useSkillHealthSummary();
  const health = useSkillHealth(skillId);
  const selected = context.data;

  return (
    <section className="grid gap-3 lg:grid-cols-2">
      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Context Budget</CardTitle>
          <select
            aria-label="上下文基准"
            value={baseline ?? ""}
            onChange={(event) =>
              setBaseline(
                event.target.value ? Number(event.target.value) : undefined,
              )
            }
            className="h-8 rounded-md border bg-background px-2 text-sm"
          >
            <option value="">未知模型</option>
            {BASELINES.map((value) => (
              <option key={value} value={value}>
                {value / 1000}K
              </option>
            ))}
          </select>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-muted-foreground text-xs">
            本地估算，不代表计费。算法：
            {contextSummary.data?.algorithm_version ?? "unicode-mixed-v1"}
          </p>
          {selected ? (
            <div className="grid grid-cols-3 gap-2 text-sm">
              {(
                [
                  ["常驻元数据", selected.metadata],
                  ["按需正文", selected.body],
                  ["辅助文件", selected.auxiliary],
                ] as const
              ).map(([label, layer]) => (
                <div key={label} className="rounded-md border p-2">
                  <div className="text-muted-foreground text-xs">{label}</div>
                  <strong>≈ {layer.estimated_tokens.toLocaleString()}</strong>
                  <div className="text-muted-foreground text-xs">
                    {layer.token_lower}–{layer.token_upper} tokens
                  </div>
                  {layer.baseline_percent != null ? (
                    <div className="mt-1 h-1.5 overflow-hidden rounded bg-muted">
                      <div
                        className="h-full bg-primary"
                        style={{
                          width: `${Math.min(layer.baseline_percent, 100)}%`,
                        }}
                      />
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : (
            <p className="text-muted-foreground text-sm">
              选择 Skill 查看分层预算。
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Health</CardTitle>
          <div className="flex gap-1 text-xs">
            <Badge variant="destructive">
              {healthSummary.data?.errors ?? 0} 错误
            </Badge>
            <Badge variant="outline">
              {healthSummary.data?.warnings ?? 0} 警告
            </Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {healthSummary.data?.completeness_status !== "complete" ? (
            <p className="rounded border border-amber-500/50 bg-amber-500/10 p-2 text-xs">
              会话索引不完整，长期未使用等结论不会标记为确定结果。
            </p>
          ) : null}
          {health.data?.checks
            .filter((item) => item.severity !== "pass")
            .map((item) => (
              <details
                key={item.check_id}
                className="rounded-md border p-2 text-sm"
              >
                <summary className="cursor-pointer font-medium">
                  <Badge
                    variant={
                      item.severity === "error" ? "destructive" : "outline"
                    }
                    className="mr-2"
                  >
                    {item.severity}
                  </Badge>
                  {item.title}
                </summary>
                <p className="mt-2 text-xs">证据：{item.evidence}</p>
                <p className="text-muted-foreground mt-1 text-xs">
                  建议：{item.recommendation}
                </p>
              </details>
            ))}
          {skillId &&
          health.data &&
          health.data.checks.every((item) => item.severity === "pass") ? (
            <p className="text-sm">所选 Skill 的静态检查全部通过。</p>
          ) : null}
          {!skillId ? (
            <p className="text-muted-foreground text-sm">
              选择 Skill 查看检查证据。
            </p>
          ) : null}
        </CardContent>
      </Card>
    </section>
  );
}
