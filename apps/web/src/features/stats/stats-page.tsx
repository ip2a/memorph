import { useState } from "react";
import { DatabaseIcon, FolderKanbanIcon, HardDriveIcon, MessageSquareIcon, RadioIcon, Rows3Icon } from "lucide-react";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { SectionHeading } from "@/components/shared/section-heading";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ActivityTrend, AttentionPanel, BreakdownTable, DistributionPanel, InactivityPanel, SessionRanking } from "@/features/stats/stats-overview-panels";
import { type StatsWorkspaceScope, useStatsDashboard } from "@/features/stats/queries";
import { formatBytes } from "@/lib/format";
import type { StatsDashboardRange } from "@/lib/types";

const rangeLabels: Record<StatsDashboardRange, string> = { "7d": "近 7 天", "30d": "近 30 天", "90d": "近 90 天", all: "全部时间" };

export function StatsPage() {
  const [range, setRange] = useState<StatsDashboardRange>("30d");
  const [scope, setScope] = useState<StatsWorkspaceScope>("workspace");
  const { dashboard, meta, all } = useStatsDashboard(range, scope);
  if (meta.isLoading || dashboard.isLoading) return <PageSkeleton />;
  if (meta.error || dashboard.error) return <PageError title="统计加载失败" message={(meta.error ?? dashboard.error)?.message ?? "未知错误"} />;
  if (!dashboard.data) return <PageError title="暂无统计数据" message="请先选择一个工作空间。" />;
  const data = dashboard.data;
  const period = rangeLabels[range];
  const metrics = [
    { label: "总会话", value: data.overview.total_sessions.toLocaleString(), hint: `累计 · ${data.overview.new_sessions} 个新增`, icon: Rows3Icon },
    { label: "活跃会话", value: data.overview.active_sessions.toLocaleString(), hint: period, icon: RadioIcon },
    { label: "消息总量", value: data.overview.total_messages.toLocaleString(), hint: `活跃会话含 ${data.overview.active_session_messages.toLocaleString()} 条`, icon: MessageSquareIcon },
    { label: "数据占用", value: formatBytes(data.overview.total_size_bytes), hint: `长期未活跃 ${formatBytes(data.overview.stale_size_bytes)}`, icon: HardDriveIcon },
    { label: "工作空间", value: data.overview.total_workspaces.toLocaleString(), hint: `${period}活跃 ${data.overview.active_workspaces}`, icon: FolderKanbanIcon },
    { label: "AI Agent", value: data.overview.total_providers.toLocaleString(), hint: `${period}活跃 ${data.overview.active_providers}`, icon: DatabaseIcon },
  ];
  return <ScrollArea className="h-full" data-stats-page><div className="flex flex-col gap-6 pb-6">
    <SectionHeading variant="page" title="会话统计" description="查看会话资产、使用趋势与需要整理的数据" actions={<div className="flex flex-wrap gap-2"><Tabs value={scope} onValueChange={(value) => setScope(value as StatsWorkspaceScope)}><TabsList><TabsTrigger value="workspace">当前工作空间</TabsTrigger><TabsTrigger value="all">全部工作空间</TabsTrigger></TabsList></Tabs><Tabs value={range} onValueChange={(value) => setRange(value as StatsDashboardRange)}><TabsList><TabsTrigger value="7d">7 天</TabsTrigger><TabsTrigger value="30d">30 天</TabsTrigger><TabsTrigger value="90d">90 天</TabsTrigger><TabsTrigger value="all">全部</TabsTrigger></TabsList></Tabs></div>} />
    <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">{metrics.map(({ icon: Icon, ...metric }) => <Card key={metric.label}><CardContent className="flex items-start gap-4 p-5"><div className="rounded-lg bg-muted p-2.5"><Icon className="size-5 text-muted-foreground"/></div><div><p className="text-sm text-muted-foreground">{metric.label}</p><p className="mt-1 text-2xl font-semibold tabular-nums">{metric.value}</p><p className="mt-1 text-xs text-muted-foreground">{metric.hint}</p></div></CardContent></Card>)}</section>
    {(data.overview.unknown_message_counts || data.overview.unknown_size_bytes || data.overview.unknown_activity_times || data.overview.unknown_created_times) ? <Card><CardContent className="p-4 text-sm text-muted-foreground">数据完整性：{data.overview.unknown_message_counts ? `${data.overview.unknown_message_counts} 个会话消息数未知` : ""}{data.overview.unknown_size_bytes ? ` · ${data.overview.unknown_size_bytes} 个会话大小未知` : ""}{data.overview.unknown_activity_times ? ` · ${data.overview.unknown_activity_times} 个会话活动时间未知` : ""}{data.overview.unknown_created_times ? ` · ${data.overview.unknown_created_times} 个会话创建时间未知` : ""}。未知值未计入对应累计统计。</CardContent></Card> : null}
    <section className="grid gap-4 lg:grid-cols-3"><ActivityTrend data={data.timeline}/><AttentionPanel data={data.attention}/></section>
    <section className="grid gap-4 lg:grid-cols-2"><InactivityPanel data={data.attention}/><BreakdownTable title="Agent 数据分布" items={data.providers}/></section>
    <section className="grid gap-4 lg:grid-cols-3"><SessionRanking data={data.top_sessions}/><BreakdownTable title={all ? "工作空间分布" : "当前工作空间 Agent"} items={all ? data.workspaces : data.providers}/></section>
    <section><SectionHeading title="数据资产" description="累计会话的体积与消息数量分布"/><div className="mt-3 grid gap-4 lg:grid-cols-2"><DistributionPanel title="会话大小分布" items={data.distributions.session_size}/><DistributionPanel title="消息数量分布" items={data.distributions.message_count}/></div></section>
  </div></ScrollArea>;
}
