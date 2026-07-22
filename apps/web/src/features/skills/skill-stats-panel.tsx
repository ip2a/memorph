import { Link, useSearchParams } from "react-router-dom";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
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
type Range = keyof typeof RANGE_DAYS;

function dateRange(range: Range) {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - RANGE_DAYS[range] + 1);
  const localDate = (date: Date) => {
    const offset = date.getTimezoneOffset() * 60_000;
    return new Date(date.getTime() - offset).toISOString().slice(0, 10);
  };
  return { from: localDate(from), to: localDate(to) };
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
    rawRange && rawRange in RANGE_DAYS ? (rawRange as Range) : "30d";
  const params: SkillStatsParams = { ...dateRange(range), provider };
  const stats = useSkillStats(params);
  const invocationQuery = useSkillInvocations(skillId, {
    ...params,
    page: 1,
    pageSize: 10,
  });
  const summary = stats.summary.data;

  return (
    <section className="grid shrink-0 gap-3 xl:grid-cols-[1fr_2fr]">
      <Card>
        <CardHeader className="flex-row items-center justify-between gap-3 pb-2">
          <CardTitle className="text-base">使用统计</CardTitle>
          <select
            aria-label="统计范围"
            className="border-input bg-background h-8 rounded-md border px-2 text-sm"
            value={range}
            onChange={(event) => {
              const next = new URLSearchParams(searchParams);
              next.set("statsRange", event.target.value);
              setSearchParams(next, { replace: true });
            }}
          >
            <option value="7d">7 天</option>
            <option value="30d">30 天</option>
            <option value="90d">90 天</option>
          </select>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-3 text-sm">
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
          {summary && summary.completeness_status !== "complete" ? (
            <Alert className="col-span-2 py-2">
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
        <CardContent className="h-40">
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
      <Card className="xl:col-span-2">
        <CardHeader className="pb-2">
          <CardTitle className="text-base">调用证据</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 xl:grid-cols-2">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Skill</TableHead>
                <TableHead>调用</TableHead>
                <TableHead>会话</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {(stats.ranking.data ?? []).slice(0, 8).map((item) => (
                <TableRow key={item.skill_id}>
                  <TableCell>{item.name}</TableCell>
                  <TableCell>{item.invocations}</TableCell>
                  <TableCell>{item.sessions}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <div className="max-h-48 space-y-2 overflow-auto">
            {(invocationQuery.data?.items ?? []).map((item) => (
              <div key={item.id} className="rounded-md border p-2 text-xs">
                <div className="flex flex-wrap items-center gap-2">
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
                <p className="text-muted-foreground mt-1 line-clamp-2">
                  {item.evidence_text || item.evidence_path || "无证据摘要"}
                </p>
              </div>
            ))}
            {skillId && invocationQuery.data?.items.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                所选 Skill 在该范围内没有调用证据。
              </p>
            ) : null}
          </div>
        </CardContent>
      </Card>
    </section>
  );
}
