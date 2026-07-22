import { useEffect, useMemo, useState } from "react";
import {
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { CopyIcon, RefreshCwIcon, SearchIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SectionHeading } from "@/components/shared/section-heading";
import { TwoPanePage } from "@/components/shared/two-pane-page";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useInstallSkill,
  useScanSkills,
  useSkillDetail,
  useSkillFilePreview,
  useSkillTree,
  useSkills,
  useUninstallSkill,
} from "@/features/skills/queries";
import { SkillBundlePanel } from "@/features/skills/skill-bundle-panel";
import { SkillStatsPanel } from "@/features/skills/skill-stats-panel";
import { SkillContextHealthPanel } from "@/features/skills/skill-context-health-panel";
import { SkillGraphPanel } from "@/features/skills/skill-graph-panel";
import { SkillPrunePanel } from "@/features/skills/skill-prune-panel";
import { SkillCoverageConflictsPanel } from "@/features/skills/skill-coverage-conflicts-panel";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SkillCatalogItem } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";

const AGENTS = ["claude", "codex", "gemini", "opencode", "hermes"] as const;

async function copyFingerprint(value: string) {
  try {
    await navigator.clipboard.writeText(value);
    toast.success("已复制指纹");
  } catch {
    toast.error("复制指纹失败");
  }
}

export function SkillsPage() {
  const { t } = useI18n();
  const [search, setSearch] = useState("");
  const [provider, setProvider] = useState("all");
  const [scope, setScope] = useState("all");
  const [sort, setSort] = useState<"name" | "size" | "files" | "updated">(
    "name",
  );
  const [order, setOrder] = useState<"asc" | "desc">("asc");
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [sourceProvider, setSourceProvider] = useState<string>();
  const [removalProvider, setRemovalProvider] = useState<string | null>(null);
  const skillsQuery = useSkills({
    query: search.trim() || undefined,
    provider: provider === "all" ? undefined : provider,
    scope: scope === "all" ? undefined : (scope as "global" | "project"),
    sort,
    order,
    page,
    pageSize: 50,
  });
  const currentWorkspace = useUiStore((state) => state.selectedWorkspace) ?? undefined;
  const scanMutation = useScanSkills();
  const installMutation = useInstallSkill();
  const uninstallMutation = useUninstallSkill();
  const items = skillsQuery.data?.items ?? [];
  const selected =
    items.find((item) => item.id === selectedId) ?? items[0] ?? null;
  const sourceId = selected?.source_id ?? null;
  const selectedSource = selected?.installations.some(
    (item) => item.provider_id === sourceProvider,
  )
    ? sourceProvider
    : selected?.installations[0]?.provider_id;
  const detailQuery = useSkillDetail(sourceId);
  const treeQuery = useSkillTree(sourceId);
  const previewQuery = useSkillFilePreview(
    sourceId,
    previewPath,
    selectedSource,
  );

  useEffect(() => setPage(1), [search, provider, scope, sort, order]);
  useEffect(() => {
    setPreviewPath(null);
    setSourceProvider(undefined);
  }, [selected?.id]);
  useEffect(() => {
    const assets = treeQuery.data?.assets ?? [];
    if (previewPath && assets.some((asset) => asset.path === previewPath))
      return;
    setPreviewPath(
      (assets.find((asset) => asset.previewable) ?? assets[0])?.path ?? null,
    );
  }, [previewPath, treeQuery.data?.assets]);

  const columns = useMemo<ColumnDef<SkillCatalogItem>[]>(
    () => [
      { accessorKey: "name", header: "Skill" },
      { accessorKey: "file_count", header: "文件" },
      { accessorKey: "total_bytes", header: "大小" },
    ],
    [],
  );
  const table = useReactTable({
    data: items,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  if (skillsQuery.isLoading) return <PageSkeleton />;
  if (skillsQuery.isError) {
    return (
      <PageError
        title={t("skillsLoadFailed")}
        message={skillsQuery.error.message}
        onRetry={() => skillsQuery.refetch()}
      />
    );
  }

  const pending = installMutation.isPending || uninstallMutation.isPending;
  const mutationError =
    scanMutation.error || installMutation.error || uninstallMutation.error;
  const pageCount = Math.max(1, Math.ceil((skillsQuery.data?.total ?? 0) / 50));

  return (
    <>
      <div className="flex h-full min-h-0 flex-col gap-3 overflow-auto p-1">
        <SkillStatsPanel
          skillId={selected?.id ?? null}
          provider={provider === "all" ? undefined : provider}
        />
        <SkillGraphPanel
          skillId={selected?.id ?? null}
          provider={provider === "all" ? undefined : provider}
        />
        <SkillContextHealthPanel
          skillId={selected?.id ?? null}
          provider={provider === "all" ? undefined : provider}
        />
        <SkillCoverageConflictsPanel skillId={selected?.id ?? null} />
        <SkillPrunePanel />
        <TwoPanePage className="min-h-[36rem] flex-1" data-skills-page-layout>
          <PanelCard className="flex min-h-0 flex-col gap-3 p-3">
            <section className="flex flex-col gap-3 border-b pb-3">
              <div className="flex items-center justify-between gap-2">
                <SectionHeading
                  titleAs="h1"
                  variant="page"
                  title={t("skills")}
                  className="border-b-0 pb-0"
                />
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      scanMutation.mutate(
                        { mode: "incremental", workspace: currentWorkspace },
                        {
                          onSuccess: (result) =>
                            toast.success(
                              `扫描完成：${result.skills_seen} 个 Skill，${result.installations_seen} 个安装`,
                            ),
                        },
                      )
                    }
                    disabled={scanMutation.isPending}
                  >
                    {scanMutation.isPending ? <Spinner /> : <RefreshCwIcon />}
                    增量扫描
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      if (
                        window.confirm(
                          "完整重新扫描会重新读取所有本地 Skill 和会话，但不会删除调用历史。继续吗？",
                        )
                      ) {
                        scanMutation.mutate({
                          mode: "full",
                          workspace: currentWorkspace,
                        });
                      }
                    }}
                    disabled={scanMutation.isPending}
                  >
                    完整扫描
                  </Button>
                </div>
              </div>
              {skillsQuery.data?.completeness.status !== "complete" ? (
                <div className="rounded-md border border-amber-500/50 bg-amber-500/10 px-3 py-2 text-xs">
                  会话历史仍在索引；目录列表可用，统计和清理结论暂不完整。
                </div>
              ) : null}
              <label className="relative block">
                <SearchIcon className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
                <Input
                  aria-label={t("searchSkills")}
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder={t("searchSkills")}
                  className="pl-9"
                />
              </label>
              <div className="grid grid-cols-2 gap-2">
                <select
                  aria-label="Provider"
                  value={provider}
                  onChange={(event) => setProvider(event.target.value)}
                  className="h-8 rounded-lg border bg-background px-2 text-sm"
                >
                  <option value="all">全部 Provider</option>
                  {skillsQuery.data?.providers.map((value) => (
                    <option key={value}>{value}</option>
                  ))}
                </select>
                <select
                  aria-label="安装范围"
                  value={scope}
                  onChange={(event) => setScope(event.target.value)}
                  className="h-8 rounded-lg border bg-background px-2 text-sm"
                >
                  <option value="all">全部范围</option>
                  <option value="global">全局</option>
                  <option value="project">项目</option>
                </select>
                <select
                  aria-label="排序字段"
                  value={sort}
                  onChange={(event) =>
                    setSort(event.target.value as typeof sort)
                  }
                  className="h-8 rounded-lg border bg-background px-2 text-sm"
                >
                  <option value="name">名称</option>
                  <option value="size">大小</option>
                  <option value="files">文件数</option>
                  <option value="updated">更新时间</option>
                </select>
                <select
                  aria-label="排序方向"
                  value={order}
                  onChange={(event) =>
                    setOrder(event.target.value as typeof order)
                  }
                  className="h-8 rounded-lg border bg-background px-2 text-sm"
                >
                  <option value="asc">升序</option>
                  <option value="desc">降序</option>
                </select>
              </div>
              <div className="text-muted-foreground text-xs">
                共 {skillsQuery.data?.total ?? 0} 个逻辑 Skill
              </div>
            </section>
            <ScrollPane className="flex-1">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Skill</TableHead>
                    <TableHead className="w-14">文件</TableHead>
                    <TableHead className="w-20">大小</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {table.getRowModel().rows.map((row) => {
                    const item = row.original;
                    return (
                      <TableRow
                        key={item.id}
                        data-state={
                          item.id === selected?.id ? "selected" : undefined
                        }
                        className="cursor-pointer"
                        onClick={() => setSelectedId(item.id)}
                      >
                        <TableCell>
                          <strong className="block truncate">
                            {item.name}
                          </strong>
                          <span className="text-muted-foreground block max-w-52 truncate text-xs">
                            {item.description || item.source_id}
                          </span>
                        </TableCell>
                        <TableCell>{item.file_count}</TableCell>
                        <TableCell>{formatBytes(item.total_bytes)}</TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
              {!items.length ? (
                <Empty className="border border-dashed">
                  <EmptyHeader>
                    <EmptyTitle>
                      {search ? t("skillsNoMatches") : t("skillsEmpty")}
                    </EmptyTitle>
                    <EmptyDescription>
                      {search
                        ? t("skillsNoMatchesDescription")
                        : t("skillsEmptyDescription")}
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              ) : null}
            </ScrollPane>
            <div className="flex items-center justify-between border-t pt-3 text-xs">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((value) => value - 1)}
              >
                上一页
              </Button>
              <span>
                {page} / {pageCount}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= pageCount}
                onClick={() => setPage((value) => value + 1)}
              >
                下一页
              </Button>
            </div>
          </PanelCard>

          <PanelCard className="flex min-h-0 flex-col gap-4 p-4">
            {mutationError ? (
              <PageError
                title={t("skillsActionFailed")}
                message={mutationError.message}
              />
            ) : null}
            {selected ? (
              <>
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h2 className="text-xl font-semibold">{selected.name}</h2>
                    <p className="text-muted-foreground mt-1 text-sm">
                      {selected.description || t("skillsNoDescription")}
                    </p>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => copyFingerprint(selected.bundle_hash)}
                  >
                    <CopyIcon />
                    指纹
                  </Button>
                </div>
                <ScrollPane
                  className="flex-1"
                  innerClassName="flex flex-col gap-5"
                >
                  <div className="flex flex-wrap gap-2 text-xs">
                    <Badge variant="outline">{selected.file_count} 文件</Badge>
                    <Badge variant="outline">
                      {formatBytes(selected.total_bytes)}
                    </Badge>
                    {selected.version ? (
                      <Badge variant="outline">v{selected.version}</Badge>
                    ) : null}
                    {selected.missing ? (
                      <Badge variant="destructive">缺失</Badge>
                    ) : null}
                  </div>
                  {selected.installations.length > 1 ? (
                    <label className="flex items-center gap-2 text-sm">
                      预览来源
                      <select
                        value={selectedSource}
                        onChange={(event) =>
                          setSourceProvider(event.target.value)
                        }
                        className="h-8 rounded border bg-background px-2"
                      >
                        {selected.installations.map((item) => (
                          <option
                            key={`${item.provider_id}:${item.install_path}`}
                            value={item.provider_id}
                          >
                            {item.provider_id} · {item.scope_kind}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}
                  <section className="flex flex-col gap-2">
                    <h3 className="text-sm font-semibold">Bundle 文件</h3>
                    {treeQuery.isLoading ? (
                      <Spinner />
                    ) : (
                      <SkillBundlePanel
                        skillId={sourceId}
                        assets={treeQuery.data?.assets ?? []}
                        previewPath={previewPath}
                        onPreviewPathChange={setPreviewPath}
                        preview={previewQuery.data}
                        previewLoading={previewQuery.isLoading}
                      />
                    )}
                  </section>
                  <section className="flex flex-col gap-3">
                    <h3 className="text-sm font-semibold">安装实例</h3>
                    {AGENTS.map((agent) => {
                      const installation = selected.installations.find(
                        (item) =>
                          item.provider_id === agent &&
                          item.status === "active",
                      );
                      const managed =
                        installation?.install_kind === "symlink" ||
                        installation?.install_kind === "managed-copy";
                      return (
                        <div
                          key={agent}
                          className="flex items-center justify-between gap-3 rounded-lg border p-3"
                        >
                          <div className="min-w-0">
                            <div className="flex gap-2">
                              <strong>{agent}</strong>
                              <Badge
                                variant={installation ? "secondary" : "outline"}
                              >
                                {installation
                                  ? t("installed")
                                  : t("notInstalled")}
                              </Badge>
                              {installation ? (
                                <Badge variant="outline">
                                  {installation.scope_kind}
                                </Badge>
                              ) : null}
                            </div>
                            <p className="text-muted-foreground mt-1 truncate font-mono text-xs">
                              {installation?.install_path ?? "—"}
                            </p>
                          </div>
                          {installation ? (
                            <Button
                              variant="destructive"
                              size="sm"
                              disabled={pending || !managed}
                              title={
                                !managed ? t("skillsUserOwnedHint") : undefined
                              }
                              onClick={() => setRemovalProvider(agent)}
                            >
                              <Trash2Icon />
                              {t("remove")}
                            </Button>
                          ) : (
                            <Button
                              size="sm"
                              disabled={pending || !selectedSource}
                              onClick={() =>
                                installMutation.mutate({
                                  skill_id: selected.source_id,
                                  provider: agent,
                                  source_provider: selectedSource,
                                })
                              }
                            >
                              {t("install")}
                            </Button>
                          )}
                        </div>
                      );
                    })}
                  </section>
                  {detailQuery.data ? (
                    <section className="rounded-lg border p-3">
                      <h3 className="text-sm font-semibold">元数据</h3>
                      <div className="mt-2 grid gap-1 text-xs">
                        {Object.entries(detailQuery.data.frontmatter).map(
                          ([key, value]) => (
                            <div key={key}>
                              <span className="text-muted-foreground">
                                {key}:
                              </span>{" "}
                              {value}
                            </div>
                          ),
                        )}
                      </div>
                    </section>
                  ) : null}
                </ScrollPane>
              </>
            ) : (
              <Empty>
                <EmptyHeader>
                  <EmptyTitle>{t("skillsEmpty")}</EmptyTitle>
                </EmptyHeader>
              </Empty>
            )}
          </PanelCard>
        </TwoPanePage>
      </div>
      <AlertDialog
        open={Boolean(removalProvider)}
        onOpenChange={(open) => !open && setRemovalProvider(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveDescription", {
                skill: selected?.name || "",
                agent: removalProvider || "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (selected && removalProvider)
                  uninstallMutation.mutate({
                    skill_id: selected.source_id,
                    provider: removalProvider,
                  });
                setRemovalProvider(null);
              }}
            >
              {t("remove")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
