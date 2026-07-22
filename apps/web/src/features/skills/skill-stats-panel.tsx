import { Link, useSearchParams } from "react-router-dom";
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
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useSkillInvocations, useSkillStats } from "@/features/skills/queries";
import type { SkillStatsParams } from "@/lib/types";

const RANGE_DAYS = { "7d": 7, "30d": 30, "90d": 90 } as const;
type Range = keyof typeof RANGE_DAYS | "custom";

function localDate(date: Date) {
  const offset = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offset).toISOString().slice(0, 10);
}

function presetRange(range: keyof typeof RANGE_DAYS) {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - RANGE_DAYS[range] + 1);
  return { from: localDate(from), to: localDate(to) };
}

function formatTime(value?: number | null) {
  return value ? new Date(value).toLocaleString() : "—";
}

function Sparkline({ values }: { values: number[] }) {
  if (values.length < 2)
    return <span className="text-muted-foreground">—</span>;
  const max = Math.max(...values, 1);
  const points = values
    .map(
      (value, index) =>
        `${(index / (values.length - 1)) * 64},${20 - (value / max) * 18}`,
    )
    .join(" ");
  return (
    <svg aria-label="调用趋势" className="h-5 w-16" viewBox="0 0 64 20">
      <polyline
        fill="none"
        points={points}
        stroke="currentColor"
        strokeWidth="1.5"
      />
    </svg>
  );
}

export function SkillStatsPanel({
  skillId,
  provider,
}: {
  skillId: string | null;
  provider?: string;
}) {
  const [searchParams, setSearchParams] = useSearchParams();
  const rawRange = searchParams.get("statsRange");
  const range: Range =
    rawRange === "custom" || (rawRange && rawRange in RANGE_DAYS)
      ? (rawRange as Range)
      : "30d";
  const dates =
    range === "custom"
      ? {
          from: searchParams.get("statsFrom") || localDate(new Date()),
          to: searchParams.get("statsTo") || localDate(new Date()),
        }
      : presetRange(range);
  const confidence = searchParams.get("statsConfidence") || undefined;
  const page = Math.max(1, Number(searchParams.get("statsPage")) || 1);
  const params: SkillStatsParams = {
    ...dates,
    provider,
    confidence: confidence as SkillStatsParams["confidence"],
  };
  const stats = useSkillStats(params);
  const invocationQuery = useSkillInvocations(skillId, {
    ...params,
    page,
    pageSize: 10,
  });
  const summary = stats.summary.data;
  const invocations = invocationQuery.data;
  const update = (values: Record<string, string | null>) => {
    const next = new URLSearchParams(searchParams);
    Object.entries(values).forEach(([key, value]) =>
      value ? next.set(key, value) : next.delete(key),
    );
    setSearchParams(next, { replace: true });
  };

  return (
    <section className="grid shrink-0 gap-3 xl:grid-cols-2">
      <Card>
        <CardHeader className="gap-3 pb-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <CardTitle className="text-base">使用统计</CardTitle>
            <div className="flex flex-wrap gap-2">
              <select
                aria-label="统计范围"
                className="border-input bg-background h-8 rounded-md border px-2 text-sm"
                value={range}
                onChange={(event) =>
                  update({ statsRange: event.target.value, statsPage: null })
                }
              >
                <option value="7d">7 天</option>
                <option value="30d">30 天</option>
                <option value="90d">90 天</option>
                <option value="custom">自定义</option>
              </select>
              <select
                aria-label="置信度"
                className="border-input bg-background h-8 rounded-md border px-2 text-sm"
                value={confidence || ""}
                onChange={(event) =>
                  update({
                    statsConfidence: event.target.value || null,
                    statsPage: null,
                  })
                }
              >
                <option value="">全部置信度</option>
                <option value="high">高</option>
                <option value="medium">中</option>
                <option value="low">低</option>
              </select>
            </div>
          </div>
          {range === "custom" ? (
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <input
                aria-label="开始日期"
                className="border-input bg-background h-8 rounded-md border px-2"
                type="date"
                value={dates.from}
                onChange={(event) =>
                  update({ statsFrom: event.target.value, statsPage: null })
                }
              />
              <span>至</span>
              <input
                aria-label="结束日期"
                className="border-input bg-background h-8 rounded-md border px-2"
                type="date"
                value={dates.to}
                onChange={(event) =>
                  update({ statsTo: event.target.value, statsPage: null })
                }
              />
            </div>
          ) : null}
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
          <div>
            <strong className="block text-xl">
              {summary?.invocations ?? "—"}
            </strong>
            调用
          </div>
          <div>
            <strong className="block text-xl">
              {summary?.active_skills ?? "—"}
            </strong>
            活跃 Skill
          </div>
          <div>
            <strong className="block text-xl">
              {summary?.active_sessions ?? "—"}
            </strong>
            会话
          </div>
          <div>
            <strong className="block text-xl">
              {summary?.active_days ?? "—"}
            </strong>
            活跃天
          </div>
          <div className="col-span-2">
            <strong className="block text-sm">
              {formatTime(summary?.last_invoked_at_ms)}
            </strong>
            最近调用
          </div>
          {summary && summary.completeness_status !== "complete" ? (
            <Alert className="col-span-full py-2">
              <AlertTitle>索引尚未完整</AlertTitle>
              <AlertDescription>
                当前统计可能继续增长，清理结论应保持禁用。
              </AlertDescription>
            </Alert>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">每日调用趋势</CardTitle>
        </CardHeader>
        <CardContent className="h-44">
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
        </CardContent>
      </Card>

      {(["providers", "workspaces"] as const).map((kind) => (
        <Card key={kind}>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">
              {kind === "providers" ? "Provider 分布" : "项目分布"}
            </CardTitle>
          </CardHeader>
          <CardContent className="h-44">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart
                data={stats.breakdown.data?.[kind] ?? []}
                layout="vertical"
                margin={{ left: 18, right: 8 }}
              >
                <CartesianGrid strokeDasharray="3 3" horizontal={false} />
                <XAxis type="number" allowDecimals={false} />
                <YAxis
                  type="category"
                  dataKey="key"
                  width={90}
                  tick={{ fontSize: 11 }}
                />
                <Tooltip />
                <Bar
                  dataKey="invocations"
                  fill="var(--primary)"
                  radius={[0, 3, 3, 0]}
                />
              </BarChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
      ))}

      <Card className="xl:col-span-2">
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Skill 排名与调用证据</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4 xl:grid-cols-[2fr_3fr]">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Skill</TableHead>
                <TableHead>调用</TableHead>
                <TableHead>会话</TableHead>
                <TableHead>最近</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {(stats.ranking.data ?? []).slice(0, 8).map((item) => (
                <TableRow key={item.skill_id}>
                  <TableCell>{item.name}</TableCell>
                  <TableCell>{item.invocations}</TableCell>
                  <TableCell>{item.sessions}</TableCell>
                  <TableCell className="text-xs">
                    {formatTime(item.last_invoked_at_ms)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <div className="space-y-2">
            {!skillId ? (
              <p className="text-muted-foreground text-sm">
                选择 Skill 后查看调用证据。
              </p>
            ) : null}
            {(invocations?.items ?? []).map((item) => (
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
                    打开会话
                  </Link>
                </div>
                <p className="text-muted-foreground mt-1">
                  项目：{item.workspace_dir || "未指定"}
                </p>
                <p className="text-muted-foreground mt-1 line-clamp-2">
                  {item.evidence_text || item.evidence_path || "无证据摘要"}
                </p>
              </div>
            ))}
            {skillId && invocations?.items.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                所选 Skill 在该范围内没有调用证据。
              </p>
            ) : null}
            {skillId &&
            invocations &&
            invocations.total > invocations.page_size ? (
              <div className="flex items-center justify-end gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={page <= 1}
                  onClick={() => update({ statsPage: String(page - 1) })}
                >
                  上一页
                </Button>
                <span className="text-xs">第 {page} 页</span>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={page * invocations.page_size >= invocations.total}
                  onClick={() => update({ statsPage: String(page + 1) })}
                >
                  下一页
                </Button>
              </div>
            ) : null}
          </div>
        </CardContent>
      </Card>
    </section>
  );
}
