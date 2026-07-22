import { useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronDownIcon,
  CopyIcon,
  RefreshCwIcon,
  SearchIcon,
  StarIcon,
  Trash2Icon,
} from "lucide-react";
import { toast } from "sonner";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SectionHeading } from "@/components/shared/section-heading";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  useInstallSkill,
  useSkillAnalysis,
  useSkillDetail,
  useSkillFilePreview,
  useSkillTree,
  useSkills,
  useUninstallSkill,
} from "@/features/skills/queries";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAgent, SkillEntry } from "@/lib/types";
import { cn } from "@/lib/utils";
import { SkillBundlePanel } from "@/features/skills/skill-bundle-panel";

function SkillDescription({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);
  const measureRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setExpanded(false);
  }, [text]);

  useEffect(() => {
    if (expanded) return;
    const el = measureRef.current;
    if (!el) return;
    const check = () => setOverflows(el.scrollHeight > el.clientHeight + 1);
    check();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(check);
    observer.observe(el);
    return () => observer.disconnect();
  }, [text, expanded]);

  return (
    <div className="mt-2 grid">
      {!expanded ? (
        <div
          ref={measureRef}
          aria-hidden
          className="text-muted-foreground invisible col-start-1 row-start-1 text-sm line-clamp-2"
        >
          {text}
        </div>
      ) : null}
      <div
        className={cn(
          "text-muted-foreground col-start-1 row-start-1 text-sm",
          !expanded && "line-clamp-2",
        )}
      >
        {text}
        {overflows ? (
          <>
            {" "}
            <button
              type="button"
              className="text-foreground inline-flex items-center gap-0.5 align-baseline text-xs font-medium hover:underline"
              onClick={() => setExpanded((value) => !value)}
            >
              {expanded ? "收起" : "展开"}
              <ChevronDownIcon
                className={cn(
                  "size-3.5 transition-transform",
                  expanded && "rotate-180",
                )}
              />
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}

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
  const skillsQuery = useSkills();
  const analysisQuery = useSkillAnalysis();
  const installMutation = useInstallSkill();
  const uninstallMutation = useUninstallSkill();
  const [search, setSearch] = useState("");
  const [scope, setScope] = useState("all");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [favorites, setFavorites] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("memorph.skill-favorites") || "[]");
    } catch {
      return [];
    }
  });
  const [collections, setCollections] = useState<Record<string, string>>(() => {
    try {
      return JSON.parse(localStorage.getItem("memorph.skill-collections") || "{}");
    } catch {
      return {};
    }
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [sourceProvider, setSourceProvider] = useState<string | undefined>();
  const [inspectionOpen, setInspectionOpen] = useState(false);
  const [removal, setRemoval] = useState<{
    skill: SkillEntry;
    agent: SkillAgent;
  } | null>(null);

  const skills = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return (skillsQuery.data?.skills || []).filter((skill) => {
      if (
        scope !== "all" &&
        !skill.installations.some((item) => item.provider_id === scope)
      ) {
        return false;
      }
      if (favoritesOnly && !favorites.includes(skill.id)) return false;
      return !query ||
        `${skill.name} ${skill.description || ""} ${collections[skill.id] || ""}`
          .toLocaleLowerCase()
          .includes(query);
    });
  }, [collections, favorites, favoritesOnly, scope, search, skillsQuery.data?.skills]);

  const selected =
    skills.find((skill) => skill.id === selectedId) || skills[0] || null;
  const selectedUsage = analysisQuery.data?.skills.find(
    (usage) => usage.skill_id === selected?.id,
  );
  const favorite = selected ? favorites.includes(selected.id) : false;

  function toggleFavorite(skillId: string) {
    const next = favorites.includes(skillId)
      ? favorites.filter((id) => id !== skillId)
      : [...favorites, skillId];
    setFavorites(next);
    localStorage.setItem("memorph.skill-favorites", JSON.stringify(next));
  }

  function setCollection(skillId: string, value: string) {
    const next = { ...collections, [skillId]: value };
    if (!value.trim()) delete next[skillId];
    setCollections(next);
    localStorage.setItem("memorph.skill-collections", JSON.stringify(next));
  }
  const selectedSource = selected?.installations.some(
    (item) => item.provider_id === sourceProvider,
  )
    ? sourceProvider
    : selected?.installations[0]?.provider_id;
  const detailQuery = useSkillDetail(selected?.id || null);
  const treeQuery = useSkillTree(selected?.id || null);
  const previewQuery = useSkillFilePreview(
    selected?.id || null,
    previewPath,
    selectedSource,
  );

  useEffect(() => {
    setPreviewPath(null);
    setInspectionOpen(false);
  }, [selected?.id]);

  useEffect(() => {
    const assets = treeQuery.data?.assets ?? [];
    if (previewPath && assets.some((asset) => asset.path === previewPath))
      return;
    const next = assets.find((asset) => asset.previewable) ?? assets[0];
    setPreviewPath(next?.path ?? null);
  }, [previewPath, selected?.id, treeQuery.data?.assets]);

  if (skillsQuery.isLoading) return <PageSkeleton />;
  if (skillsQuery.isError) {
    return (
      <PageError
        title={t("skillsLoadFailed")}
        message={
          skillsQuery.error instanceof Error
            ? skillsQuery.error.message
            : t("skillsLoadFailed")
        }
        onRetry={() => skillsQuery.refetch()}
      />
    );
  }

  const pending = installMutation.isPending || uninstallMutation.isPending;
  const mutationError = installMutation.error || uninstallMutation.error;

  const refreshButton = (
    <Button
      type="button"
      variant="outline"
      onClick={() => skillsQuery.refetch()}
      disabled={skillsQuery.isFetching}
    >
      {skillsQuery.isFetching ? (
        <Spinner data-icon="inline-start" />
      ) : (
        <RefreshCwIcon data-icon="inline-start" />
      )}
      {t("refresh")}
    </Button>
  );

  return (
    <>
      <TwoPanePage className="h-full min-h-0" data-skills-page-layout>
        <PanelCard className="flex min-h-0 flex-col gap-3 p-3">
          <section className="flex flex-col gap-3 border-b pb-3">
            <SectionHeading
              titleAs="h1"
              variant="page"
              title={t("skills")}
              className="border-b-0 pb-0"
            />
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
            <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2">
              <select
                aria-label="Skill 范围"
                value={scope}
                onChange={(event) => setScope(event.target.value)}
                className="h-8 rounded-lg border bg-background px-2 text-sm"
              >
                <option value="all">全部范围</option>
                {skillsQuery.data?.agents.map((agent) => (
                  <option key={agent.provider_id} value={agent.provider_id}>
                    {agent.name}
                  </option>
                ))}
              </select>
              <Button
                type="button"
                size="sm"
                variant={favoritesOnly ? "secondary" : "outline"}
                onClick={() => setFavoritesOnly((value) => !value)}
              >
                <StarIcon data-icon="inline-start" />
                收藏
              </Button>
            </div>
            <div className="grid grid-cols-3 gap-2 text-center text-xs">
              <div className="rounded-md border p-2">
                <strong className="block text-base">
                  {analysisQuery.data?.scanned_sessions ?? 0}
                </strong>
                已扫描会话
              </div>
              <div className="rounded-md border p-2">
                <strong className="block text-base">
                  {analysisQuery.data?.invocations ?? 0}
                </strong>
                调用
              </div>
              <div className="rounded-md border p-2">
                <strong className="block text-base">
                  {analysisQuery.data?.skills.filter((item) => item.prune_candidate)
                    .length ?? 0}
                </strong>
                清理建议
              </div>
              <div className="rounded-md border p-2">
                <strong className="block text-base">
                  {analysisQuery.data?.total_tokens?.toLocaleString() ?? 0}
                </strong>
                Token（令牌）
              </div>
              <div className="rounded-md border p-2">
                <strong className="block text-base">
                  {analysisQuery.data?.hook_sessions ?? 0}
                </strong>
                Hook（钩子）会话
              </div>
            </div>
            <div className="text-muted-foreground flex items-center justify-between text-xs">
              <span>{t("skillsFound", { count: skills.length })}</span>
              <span>
                {t("skillsAgents", {
                  count: skillsQuery.data?.agents.length || 0,
                })}
              </span>
            </div>
          </section>
          <ScrollPane className="flex-1" innerClassName="flex flex-col gap-2">
            {skills.map((skill) => (
              <SelectableRowButton
                key={skill.id}
                selected={skill.id === selectedId}
                onClick={() => setSelectedId(skill.id)}
                title={skill.name}
                meta={skill.description || skill.directory}
                trailing={
                  <Badge variant="outline">{skill.statistics.files}</Badge>
                }
              />
            ))}
            {!skills.length ? (
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
        </PanelCard>

        <PanelCard className="flex min-h-0 flex-col gap-4 p-4">
          {mutationError ? (
            <PageError
              title={t("skillsActionFailed")}
              message={
                mutationError instanceof Error
                  ? mutationError.message
                  : t("skillsActionFailed")
              }
            />
          ) : null}
          {selected ? (
            <>
              <div className="flex shrink-0 items-center justify-between gap-3">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <h2 className="text-xl font-semibold">{selected.name}</h2>
                  <Badge variant="secondary">{selected.directory}</Badge>
                  {collections[selected.id] ? (
                    <Badge variant="outline">{collections[selected.id]}</Badge>
                  ) : null}
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    variant={favorite ? "secondary" : "outline"}
                    size="icon-sm"
                    aria-label={favorite ? "取消收藏" : "收藏 Skill"}
                    onClick={() => toggleFavorite(selected.id)}
                  >
                    <StarIcon className={favorite ? "fill-current" : undefined} />
                  </Button>
                  {refreshButton}
                </div>
              </div>
              <ScrollPane
                className="flex-1"
                innerClassName="flex flex-col gap-5"
              >
                <SkillDescription
                  text={selected.description || t("skillsNoDescription")}
                />

                <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">调用次数</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.invocations ?? 0}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">涉及会话</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.sessions ?? 0}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">Token（令牌）</span>
                    <strong className="mt-1 block text-xl">
                      {(selectedUsage?.total_tokens ?? 0).toLocaleString()}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">Hook（钩子）观测</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.hook_observed ? "是" : "否"}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">成本</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.estimated_cost_usd == null
                        ? "未记录"
                        : `$${selectedUsage.estimated_cost_usd.toFixed(4)}`}
                    </strong>
                  </div>
                </section>

                <label className="flex items-center gap-3 rounded-lg border p-3 text-sm">
                  <span className="shrink-0 font-medium">集合</span>
                  <Input
                    aria-label="Skill 集合"
                    value={collections[selected.id] || ""}
                    onChange={(event) => setCollection(selected.id, event.target.value)}
                    placeholder="例如：文档、开发、设计"
                  />
                </label>

                <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">健康分</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.health_score ?? 100}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">上下文预算</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.context_tokens.toLocaleString() ?? 0}
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">覆盖率</span>
                    <strong className="mt-1 block text-xl">
                      {(selectedUsage?.coverage_percent ?? 0).toFixed(0)}%
                    </strong>
                  </div>
                  <div className="rounded-lg border p-3">
                    <span className="text-muted-foreground text-xs">清理建议</span>
                    <strong className="mt-1 block text-xl">
                      {selectedUsage?.prune_candidate ? "未使用" : "保留"}
                    </strong>
                  </div>
                </section>

                {selectedUsage?.traces.length ? (
                  <section className="flex flex-col gap-2">
                    <h3 className="text-sm font-semibold">最近调用追踪</h3>
                    <div className="divide-y rounded-lg border">
                      {selectedUsage.traces.slice(0, 5).map((trace) => (
                        <div
                          key={`${trace.session_id}-${trace.event_id}`}
                          className="p-3 text-xs"
                        >
                          <div className="flex items-center justify-between gap-3">
                            <strong className="truncate">
                              {trace.session_title || trace.session_id}
                            </strong>
                            <Badge variant="outline">{trace.provider_id}</Badge>
                          </div>
                          <p className="text-muted-foreground mt-1">
                            {new Date(trace.timestamp).toLocaleString()} · {trace.source}
                          </p>
                        </div>
                      ))}
                    </div>
                  </section>
                ) : null}

                <div className="flex flex-nowrap items-center gap-2 overflow-x-auto text-xs">
                  <Badge variant="outline" className="shrink-0">
                    <span className="text-muted-foreground">文件</span>
                    {detailQuery.data?.statistics.files ??
                      selected.statistics.files}
                  </Badge>
                  <Badge variant="outline" className="shrink-0">
                    <span className="text-muted-foreground">大小</span>
                    {formatBytes(
                      detailQuery.data?.statistics.bytes ??
                        selected.statistics.bytes,
                    )}
                  </Badge>
                  <Badge
                    asChild
                    variant="outline"
                    className="shrink-0 cursor-pointer hover:bg-muted"
                  >
                    <button
                      type="button"
                      onClick={() =>
                        copyFingerprint(
                          detailQuery.data?.fingerprint ?? selected.fingerprint,
                        )
                      }
                    >
                      <CopyIcon data-icon="inline-start" />
                      指纹
                    </button>
                  </Badge>
                  <Badge
                    asChild
                    variant="outline"
                    className="shrink-0 cursor-pointer hover:bg-muted"
                  >
                    <button
                      type="button"
                      onClick={() => setInspectionOpen(true)}
                    >
                      <span className="text-muted-foreground">
                        {t("skillsAnalysis")}
                      </span>
                      {detailQuery.data?.frontmatter.version ||
                        t("skillsUndeclared")}
                      {selected.issues.length ? (
                        <span className="text-amber-600 dark:text-amber-400">
                          · {selected.issues.length}
                        </span>
                      ) : null}
                    </button>
                  </Badge>
                </div>
                {analysisQuery.data?.trigger_conflicts?.some((item) =>
                  item.skills.includes(selected.id),
                ) ? (
                  <div className="rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-sm">
                    Trigger Conflict（触发冲突）：该 Skill 与其他 Skill 使用了相同触发词。
                  </div>
                ) : null}
                {selected.conflict ? (
                  <div className="rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-sm">
                    多来源内容存在冲突。安装时必须明确选择来源。
                    <select
                      className="mt-2 block rounded border bg-background p-1"
                      value={selectedSource || ""}
                      onChange={(event) =>
                        setSourceProvider(event.target.value)
                      }
                    >
                      {selected.installations.map((item) => (
                        <option key={item.provider_id} value={item.provider_id}>
                          {item.provider_id} · {item.fingerprint}
                        </option>
                      ))}
                    </select>
                  </div>
                ) : null}
                <section className="flex flex-col gap-2">
                  <h3 className="text-sm font-semibold">Bundle 文件</h3>
                  <SkillBundlePanel
                    assets={treeQuery.data?.assets ?? []}
                    previewPath={previewPath}
                    onPreviewPathChange={setPreviewPath}
                    preview={previewQuery.data}
                    previewLoading={
                      previewQuery.isFetching && Boolean(previewPath)
                    }
                  />
                </section>

                <section className="flex flex-col gap-3">
                  <h3 className="text-sm font-semibold">
                    {t("skillsInstallations")}
                  </h3>
                  {skillsQuery.data?.agents.map((agent) => {
                    const installation = selected.installations.find(
                      (item) => item.provider_id === agent.provider_id,
                    );
                    const acting =
                      pending &&
                      ((installMutation.variables?.skill_id === selected.id &&
                        installMutation.variables.provider ===
                          agent.provider_id) ||
                        (uninstallMutation.variables?.skill_id ===
                          selected.id &&
                          uninstallMutation.variables.provider ===
                            agent.provider_id));
                    return (
                      <div
                        key={agent.provider_id}
                        className="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3"
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <strong className="text-sm">{agent.name}</strong>
                            <Badge
                              variant={installation ? "secondary" : "outline"}
                            >
                              {installation
                                ? t("installed")
                                : t("notInstalled")}
                            </Badge>
                            {installation ? (
                              <Badge variant="outline">
                                {installation.deployment_mode === "symlink"
                                  ? "符号链接"
                                  : installation.deployment_mode === "copy"
                                    ? "受管复制"
                                    : "外部目录"}
                              </Badge>
                            ) : null}
                            {installation && !installation.link_valid ? (
                              <Badge variant="destructive">链接失效</Badge>
                            ) : null}
                          </div>
                          <p className="text-muted-foreground mt-1 truncate font-mono text-xs">
                            {installation?.path || agent.skills_dir}
                          </p>
                        </div>
                        {installation ? (
                          <Button
                            type="button"
                            variant="destructive"
                            size="sm"
                            disabled={pending || !installation.managed}
                            title={
                              !installation.managed
                                ? t("skillsUserOwnedHint")
                                : undefined
                            }
                            onClick={() =>
                              setRemoval({ skill: selected, agent })
                            }
                          >
                            {acting ? (
                              <Spinner />
                            ) : (
                              <Trash2Icon data-icon="inline-start" />
                            )}
                            {t("remove")}
                          </Button>
                        ) : (
                          <Button
                            type="button"
                            size="sm"
                            disabled={pending}
                            onClick={() =>
                              installMutation.mutate({
                                skill_id: selected.id,
                                provider: agent.provider_id,
                                source_provider: selectedSource,
                              })
                            }
                          >
                            {acting ? <Spinner /> : null}
                            {t("install")}
                          </Button>
                        )}
                      </div>
                    );
                  })}
                </section>
              </ScrollPane>
            </>
          ) : (
            <div className="flex min-h-0 flex-1 flex-col gap-4">
              <div className="flex shrink-0 justify-end">{refreshButton}</div>
              <Empty className="flex-1">
                <EmptyHeader>
                  <EmptyTitle>{t("skillsSelect")}</EmptyTitle>
                  <EmptyDescription>
                    {t("skillsSelectDescription")}
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            </div>
          )}
        </PanelCard>
      </TwoPanePage>

      <Dialog open={inspectionOpen} onOpenChange={setInspectionOpen}>
        <DialogContent className="sm:max-w-lg" data-skill-inspection-dialog>
          <DialogHeader>
            <DialogTitle>{t("skillsAnalysis")}</DialogTitle>
            <DialogDescription>
              {selected?.name || t("skills")}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-4 text-sm">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-md border p-3">
                <span className="text-muted-foreground block text-xs">
                  {t("skillsVersion")}
                </span>
                <strong className="mt-1 block">
                  {detailQuery.data?.frontmatter.version ||
                    t("skillsUndeclared")}
                </strong>
              </div>
              <div className="rounded-md border p-3 sm:col-span-2">
                <span className="text-muted-foreground block text-xs">
                  {t("skillsSource")}
                </span>
                <p className="mt-1 break-all">
                  {detailQuery.data?.frontmatter.repository ||
                    detailQuery.data?.frontmatter.source ||
                    detailQuery.data?.frontmatter.homepage ||
                    t("skillsUndeclared")}
                </p>
              </div>
            </div>
            {selected?.issues.length ? (
              <div className="rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-xs">
                <strong>
                  {t("skillsFindings", { count: selected.issues.length })}
                </strong>
                <ul className="mt-2 list-disc space-y-1 pl-5">
                  {selected.issues.map((issue, index) => (
                    <li key={`${issue.path || "bundle"}-${index}`}>
                      {issue.path ? `${issue.path}: ` : ""}
                      {issue.message}
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <p className="text-muted-foreground text-xs">
                {t("skillsNoFindings")}
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog
        open={!!removal}
        onOpenChange={(open) => !open && setRemoval(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveDescription", {
                skill: removal?.skill.name,
                agent: removal?.agent.name,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (!removal) return;
                uninstallMutation.mutate({
                  skill_id: removal.skill.id,
                  provider: removal.agent.provider_id,
                });
                setRemoval(null);
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
