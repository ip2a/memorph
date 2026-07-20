import { useState } from "react";
import { Link } from "react-router-dom";
import { Bar, BarChart, CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatBytes, formatDateTime } from "@/lib/format";
import type { StatsBreakdownItem, StatsDashboard, StatsSessionItem } from "@/lib/types";

const chartConfig = {
  active_sessions: { label: "活跃会话", color: "var(--chart-1)" },
  new_sessions: { label: "新增会话", color: "var(--chart-2)" },
  active_session_messages: { label: "活跃会话消息", color: "var(--chart-3)" },
} satisfies ChartConfig;

type TrendKey = keyof typeof chartConfig;
type RankKey = "by_messages" | "by_size" | "recently_active";

export function ActivityTrend({ data }: { data: StatsDashboard["timeline"] }) {
  const [metric, setMetric] = useState<TrendKey>("active_sessions");
  const points = data.map((point) => ({ ...point, label: new Date(point.start).toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" }) }));
  return <Card className="lg:col-span-2"><CardHeader className="flex-row items-center justify-between"><CardTitle>使用趋势</CardTitle><Tabs value={metric} onValueChange={(value) => setMetric(value as TrendKey)}><TabsList className="flex-wrap"><TabsTrigger value="active_sessions">活跃会话</TabsTrigger><TabsTrigger value="new_sessions">新增会话</TabsTrigger><TabsTrigger value="active_session_messages">会话消息</TabsTrigger><TabsTrigger value="new_size_bytes">新增数据</TabsTrigger></TabsList></Tabs></CardHeader><CardContent>{points.length ? <ChartContainer config={chartConfig} className="h-72 w-full"><LineChart accessibilityLayer data={points}><CartesianGrid vertical={false} strokeDasharray="4 4"/><XAxis dataKey="label" tickLine={false} axisLine={false}/><YAxis tickLine={false} axisLine={false} width={44}/><ChartTooltip content={<ChartTooltipContent/>}/><Line type="monotone" dataKey={metric} stroke={`var(--color-${metric})`} strokeWidth={2} dot={false}/></LineChart></ChartContainer> : <Empty/>}</CardContent></Card>;
}

export function AttentionPanel({ data }: { data: StatsDashboard["attention"] }) {
  const items = [
    ["90 天以上未活跃", data.inactive_over_90d], ["大型会话", data.large_sessions],
    ["内容较少", data.short_sessions], ["时间未知", data.unknown],
  ] as const;
  return <Card><CardHeader><CardTitle>需要关注</CardTitle></CardHeader><CardContent className="grid gap-3 sm:grid-cols-2">{items.map(([label, value]) => <div key={label} className="rounded-lg border p-4"><p className="text-sm text-muted-foreground">{label}</p><p className="mt-1 text-2xl font-semibold tabular-nums">{value.count}</p><p className="text-xs text-muted-foreground">占用 {formatBytes(value.size_bytes)}</p></div>)}</CardContent></Card>;
}

export function InactivityPanel({ data }: { data: StatsDashboard["attention"] }) {
  const items = [{ label: "7 天内", value: data.active_7d.count }, { label: "7–30 天", value: data.inactive_7_to_30d.count }, { label: "30–90 天", value: data.inactive_30_to_90d.count }, { label: "90 天以上", value: data.inactive_over_90d.count }, { label: "未知", value: data.unknown.count }];
  return <Card><CardHeader><CardTitle>会话活跃状态</CardTitle></CardHeader><CardContent>{items.some((item) => item.value) ? <ChartContainer config={{ count: { label: "会话", color: "var(--chart-1)" } }} className="h-56 w-full"><BarChart accessibilityLayer data={items} layout="vertical"><CartesianGrid horizontal={false}/><XAxis type="number" hide/><YAxis dataKey="label" type="category" tickLine={false} axisLine={false} width={78}/><ChartTooltip content={<ChartTooltipContent/>}/><Bar dataKey="value" fill="var(--color-count)" radius={4}/></BarChart></ChartContainer> : <Empty/>}</CardContent></Card>;
}

export function BreakdownTable({ title, items }: { title: string; items: StatsBreakdownItem[] }) {
  return <Card><CardHeader><CardTitle>{title}</CardTitle></CardHeader><CardContent className="p-0">{items.length ? <Table><TableHeader><TableRow><TableHead>名称</TableHead><TableHead className="text-right">会话</TableHead><TableHead className="text-right">消息</TableHead><TableHead className="text-right">空间</TableHead></TableRow></TableHeader><TableBody>{items.slice(0, 8).map((item) => <TableRow key={item.id}><TableCell className="max-w-48 truncate" title={item.id}>{item.id.split(/[\\/]/).filter(Boolean).at(-1) ?? item.id}</TableCell><TableCell className="text-right tabular-nums">{item.session_count}</TableCell><TableCell className="text-right tabular-nums">{item.message_count}</TableCell><TableCell className="text-right tabular-nums">{formatBytes(item.size_bytes)}</TableCell></TableRow>)}</TableBody></Table> : <Empty/>}</CardContent></Card>;
}

export function SessionRanking({ data }: { data: StatsDashboard["top_sessions"] }) {
  const [rank, setRank] = useState<RankKey>("by_messages"); const items = data[rank];
  return <Card className="lg:col-span-2"><CardHeader className="flex-row items-center justify-between"><CardTitle>会话排行</CardTitle><Tabs value={rank} onValueChange={(value) => setRank(value as RankKey)}><TabsList><TabsTrigger value="by_messages">消息最多</TabsTrigger><TabsTrigger value="by_size">占用最大</TabsTrigger><TabsTrigger value="recently_active">最近活跃</TabsTrigger></TabsList></Tabs></CardHeader><CardContent className="p-0">{items.length ? <SessionTable items={items}/> : <Empty/>}</CardContent></Card>;
}

function SessionTable({ items }: { items: StatsSessionItem[] }) { return <Table><TableHeader><TableRow><TableHead>会话</TableHead><TableHead>Agent</TableHead><TableHead className="text-right">消息</TableHead><TableHead className="text-right">大小</TableHead><TableHead className="text-right">最后活动</TableHead></TableRow></TableHeader><TableBody>{items.map((item) => <TableRow key={`${item.provider_id}:${item.session_id}`}><TableCell className="max-w-64 truncate"><Link className="font-medium hover:underline" to={`/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`}>{item.title}</Link></TableCell><TableCell>{item.provider_id}</TableCell><TableCell className="text-right tabular-nums">{item.message_count}</TableCell><TableCell className="text-right tabular-nums">{formatBytes(item.size_bytes)}</TableCell><TableCell className="text-right text-muted-foreground">{formatDateTime(item.last_active_at)}</TableCell></TableRow>)}</TableBody></Table>; }

export function DistributionPanel({ title, items }: { title: string; items: StatsDashboard["distributions"]["session_size"] }) { return <Card><CardHeader><CardTitle>{title}</CardTitle></CardHeader><CardContent className="space-y-3">{items.map((item) => <div key={item.key} className="flex items-center justify-between border-b pb-2 last:border-0"><span className="text-sm text-muted-foreground">{item.label}</span><span className="font-mono text-sm tabular-nums">{item.count} · {formatBytes(item.size_bytes)}</span></div>)}</CardContent></Card>; }
function Empty() { return <div className="flex min-h-32 items-center justify-center text-sm text-muted-foreground">暂无数据</div>; }
