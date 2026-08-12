import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArchiveIcon, RefreshCwIcon, SearchIcon } from "lucide-react";
import { toast } from "sonner";
import { PageError, PageSkeleton } from "@/components/shared/page-states";
import { PanelCard } from "@/components/shared/panel-card";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { SelectableRowButton } from "@/components/shared/selectable-row-button";
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
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
  useScanSkills,
  useSkillDetail,
  useSkillFilePreview,
  useSkillTree,
  useSkills,
  useUninstallSkill,
  useDeleteSkill,
  useDisableSkill,
  useConsolidateSkill,
  useRemoveSymlinksSkill,
} from "@/features/skills/queries";
import { SkillDetailPanel } from "@/features/skills/skill-detail-panel";
import { SkillDisabledDialog } from "@/features/skills/skill-disabled-dialog";
import {
  SkillsCatalogFilterTrigger,
  type SkillsCatalogFilterApply,
  type SkillsCatalogFilters,
} from "@/features/skills/skills-catalog-filters";
import { SkillOverviewPanel } from "@/features/skills/skill-overview-panel";
import { clampSkillsCatalogPageSize } from "@/features/skills/skills-catalog-page-size";
import { buildUpdateSettingsPayloadFromMeta } from "@/features/skills/skills-settings-payload";
import { getMeta, updateSettings } from "@/lib/api";
import { normalizeSkillDescription } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import { queryKeys } from "@/lib/query-keys";
import { useUiStore } from "@/stores/ui-store";
import type { SkillCatalogItem } from "@/lib/types";

// The real on-disk location of a skill: a directory/managed-copy installation's
// path, or — if every installation is a symlink — the target it points at. The
// left list shows this so two same-named copies (different real paths) can be
// told apart; symlinks never appear here as their own entry.
function realPathOf(item: SkillCatalogItem): string | undefined {
  const directory = item.installations.find(
    (installation) =>
      (installation.install_kind === "directory" ||
        installation.install_kind === "managed-copy") &&
      installation.status === "active",
  );
  if (directory) return directory.install_path;
  const symlink = item.installations.find(
    (installation) => installation.install_kind === "symlink",
  );
  return symlink?.symlink_target ?? item.installations[0]?.install_path;
}

export function SkillsPage() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [usedBy, setUsedBy] = useState("all");
  const [scope, setScope] = useState("all");
  const [sort, setSort] = useState<"name" | "size" | "files" | "updated">(
    "name",
  );
  const [order, setOrder] = useState<"asc" | "desc">("asc");
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [rightView, setRightView] = useState<"overview" | "detail">("overview");
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [removalAgent, setRemovalAgent] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [disableOpen, setDisableOpen] = useState(false);
  const [removeSymlinksOpen, setRemoveSymlinksOpen] = useState(false);
  const [disabledListOpen, setDisabledListOpen] = useState(false);
  const initialScanStarted = useRef(false);
  const metaQuery = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });
  const pageSize = clampSkillsCatalogPageSize(
    metaQuery.data?.settings.skills_catalog_page_size,
  );
  const persistPageSizeMutation = useMutation({
    mutationFn: async (nextPageSize: number) => {
      const meta = metaQuery.data ?? (await getMeta());
      await updateSettings(
        buildUpdateSettingsPayloadFromMeta(meta.settings, {
          skills_catalog_page_size: clampSkillsCatalogPageSize(nextPageSize),
        }),
      );
      return getMeta();
    },
    onSuccess: (nextMeta) => {
      queryClient.setQueryData(queryKeys.meta, nextMeta);
      void queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
    },
    onError: (error) => {
      toast.error(
        error instanceof Error ? error.message : String(error),
      );
    },
  });
  const skillsQuery = useSkills({
    query: search.trim() || undefined,
    used_by: usedBy === "all" ? undefined : usedBy,
    scope: scope === "all" ? undefined : (scope as "global" | "project"),
    sort,
    order,
    page,
    pageSize,
  });
  const currentWorkspace =
    useUiStore((state) => state.selectedWorkspace) ?? undefined;
  const scanMutation = useScanSkills();
  const installMutation = useInstallSkill();
  const uninstallMutation = useUninstallSkill();
  const deleteMutation = useDeleteSkill();
  const disableMutation = useDisableSkill();
  const consolidateMutation = useConsolidateSkill();
  const removeSymlinksMutation = useRemoveSymlinksSkill();
  const items = skillsQuery.data?.items ?? [];
  const selected = items.find((item) => item.id === selectedId) ?? null;
  const detailId = selected?.id ?? null;
  const sourceUsedBy = selected?.installations.find(
    (item) => item.status === "active",
  )?.used_by;
  const detailQuery = useSkillDetail(detailId);
  const treeQuery = useSkillTree(detailId);
  const previewQuery = useSkillFilePreview(
    detailId,
    previewPath,
    sourceUsedBy,
  );

  // Fire one background scan after the catalog first loads. The scan endpoint
  // is non-blocking, so this just queues the work — list refetches will pick up
  // the result.
  useEffect(() => {
    if (initialScanStarted.current || !skillsQuery.data) return;
    initialScanStarted.current = true;
    scanMutation.mutate({
      mode: "incremental",
      workspace: currentWorkspace,
    });
  }, [currentWorkspace, scanMutation, skillsQuery.data]);

  useEffect(() => setPage(1), [search, usedBy, scope, sort, order, pageSize]);
  useEffect(() => {
    setPreviewPath(null);
  }, [selected?.id]);
  useEffect(() => {
    const assets = treeQuery.data?.assets ?? [];
    if (previewPath && assets.some((asset) => asset.path === previewPath))
      return;
    setPreviewPath(
      (assets.find((asset) => asset.previewable) ?? assets[0])?.path ?? null,
    );
  }, [previewPath, treeQuery.data?.assets]);

  if (skillsQuery.isError) {
    return (
      <PageError
        title={t("skillsLoadFailed")}
        message={skillsQuery.error.message}
        onRetry={() => skillsQuery.refetch()}
      />
    );
  }

  const pending =
    installMutation.isPending ||
    uninstallMutation.isPending ||
    deleteMutation.isPending ||
    disableMutation.isPending ||
    consolidateMutation.isPending ||
    removeSymlinksMutation.isPending;
  const pendingAgent = installMutation.isPending
    ? (installMutation.variables?.used_by ?? null)
    : uninstallMutation.isPending
      ? (uninstallMutation.variables?.used_by ?? null)
      : null;
  const mutationError =
    scanMutation.error ||
    installMutation.error ||
    uninstallMutation.error ||
    deleteMutation.error ||
    disableMutation.error ||
    consolidateMutation.error ||
    removeSymlinksMutation.error;
  const total = skillsQuery.data?.total ?? 0;
  const responsePageSize = skillsQuery.data?.page_size ?? pageSize;
  const pageCount = Math.max(1, Math.ceil(total / responsePageSize));
  const rangeFrom = total === 0 ? 0 : (page - 1) * responsePageSize + 1;
  const rangeTo = Math.min(page * responsePageSize, total);
  const catalogFilters: SkillsCatalogFilters = {
    used_by: usedBy,
    scope: scope as SkillsCatalogFilters["scope"],
    sort,
    order,
  };

  function applyCatalogFilters(next: SkillsCatalogFilterApply) {
    setUsedBy(next.filters.used_by);
    setScope(next.filters.scope);
    setSort(next.filters.sort);
    setOrder(next.filters.order);
    const nextPageSize = clampSkillsCatalogPageSize(next.page_size);
    if (nextPageSize !== pageSize) {
      persistPageSizeMutation.mutate(nextPageSize);
    }
  }

  return (
    <>
      <div className="flex h-full min-h-0 flex-col p-1">
        <TwoPanePage className="min-h-[36rem] flex-1" data-skills-page-layout>
          <PanelCard className="flex min-h-0 flex-col gap-3 p-3">
            <section className="flex flex-col gap-3 border-b pb-3">
              <div className="flex items-center justify-between gap-2">
                <SectionHeading
                  titleAs="h1"
                  variant="page"
                  title={
                    <button
                      type="button"
                      className="cursor-pointer text-left underline-offset-4 transition-colors hover:text-primary hover:underline"
                      onClick={() => setRightView("overview")}
                    >
                      {t("skills")}
                    </button>
                  }
                  className="border-b-0 pb-0"
                />
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      uninstallMutation.reset();
                      scanMutation.mutate(
                        { mode: "incremental", workspace: currentWorkspace },
                        {
                          onSuccess: () =>
                            toast.success(t("skillsScanQueued")),
                        },
                      );
                    }}
                    disabled={scanMutation.isPending}
                    title={t("skillsRefreshList")}
                  >
                    {scanMutation.isPending ? <Spinner /> : <RefreshCwIcon />}
                    {t("skillsRefreshList")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setDisabledListOpen(true)}
                    title={t("skillsDisabledList")}
                  >
                    <ArchiveIcon />
                    {t("skillsDisabledList")}
                  </Button>
                </div>
              </div>
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
              <SkillsCatalogFilterTrigger
                pagination={{
                  rangeFrom,
                  rangeTo,
                  total,
                  page,
                  pageCount,
                  onPrevious: () => setPage((value) => value - 1),
                  onNext: () => setPage((value) => value + 1),
                }}
                pageSize={pageSize}
                filters={catalogFilters}
                usedBy={skillsQuery.data?.used_by ?? []}
                onApply={applyCatalogFilters}
              />
            </section>
            <ScrollPane className="flex-1" innerClassName="flex flex-col gap-2">
              {skillsQuery.isLoading ? (
                <div className="p-3">
                  <PageSkeleton />
                </div>
              ) : (
                items.map((item) => (
                  <SelectableRowButton
                    key={item.id}
                    selected={rightView === "detail" && item.id === selectedId}
                    title={item.name}
                    meta={
                      <span className="flex flex-col gap-0.5">
                        <span className="flex flex-wrap items-center gap-1">
                          <span>{normalizeSkillDescription(item.description) || item.source_id}</span>
                          {item.tags.map((tag) => (
                            <Badge key={tag} variant="outline">{tag}</Badge>
                          ))}
                        </span>
                        {realPathOf(item) ? (
                          <span
                            className="truncate font-mono text-[11px] text-muted-foreground"
                            title={realPathOf(item)}
                          >
                            {realPathOf(item)}
                          </span>
                        ) : null}
                      </span>
                    }
                    onClick={() => {
                      setSelectedId(item.id);
                      setRightView("detail");
                    }}
                  />
                ))
              )}
              {!skillsQuery.isLoading && !items.length ? (
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
            {rightView === "overview" ? (
              <SkillOverviewPanel
                skillId={selectedId}
                provider={usedBy === "all" ? undefined : usedBy}
              />
            ) : (
              <SkillDetailPanel
                selected={selected}
                detail={detailQuery.data}
                assets={treeQuery.data?.assets ?? []}
                treeLoading={treeQuery.isLoading}
                previewPath={previewPath}
                onPreviewPathChange={setPreviewPath}
                preview={previewQuery.data}
                previewLoading={previewQuery.isLoading}
                pending={pending}
                pendingAgent={pendingAgent}
                mutationError={mutationError}
                provider={usedBy === "all" ? undefined : usedBy}
                onInstall={(agent) => {
                  if (!selected) return;
                  installMutation.mutate(
                    {
                      skill_id: selected.source_id,
                      used_by: agent,
                      source_used_by: sourceUsedBy,
                    },
                    {
                      onSuccess: () => {
                        toast.success(
                          t("skillsInstalled", {
                            skill: selected.name,
                            agent,
                          }),
                        );
                      },
                    },
                  );
                }}
                onRemove={(agent) => {
                  uninstallMutation.reset();
                  setRemovalAgent(agent);
                }}
                onDelete={() => setDeleteOpen(true)}
                onDisable={() => setDisableOpen(true)}
                onConsolidate={(canonicalPath) => {
                  if (!selected) return;
                  consolidateMutation.mutate(canonicalPath, {
                    onSuccess: () =>
                      toast.success(
                        t("skillsConsolidated", { skill: selected.name }),
                      ),
                  });
                }}
                onRemoveSymlinks={() => setRemoveSymlinksOpen(true)}
              />
            )}
          </PanelCard>
        </TwoPanePage>
      </div>
      <AlertDialog
        open={Boolean(removalAgent)}
        onOpenChange={(open) => !open && setRemovalAgent(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveDescription", {
                skill: selected?.name || "",
                agent: removalAgent || "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={uninstallMutation.isPending}
              onClick={() => {
                if (selected && removalAgent) {
                  uninstallMutation.mutate(
                    {
                      skill_id: selected.source_id,
                      used_by: removalAgent,
                    },
                    {
                      onSuccess: () => {
                        setRemovalAgent(null);
                        toast.success(
                          t("skillsRemoved", {
                            skill: selected.name,
                            agent: removalAgent,
                          }),
                        );
                      },
                    },
                  );
                }
              }}
            >
              {uninstallMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              {t("remove")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsDeleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsDeleteDescription", { skill: selected?.name || "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (selected) {
                  deleteMutation.mutate(selected.id, {
                    onSuccess: () => {
                      setSelectedId(null);
                      setRightView("overview");
                      toast.success(t("skillsDeleted"));
                    },
                  });
                }
                setDeleteOpen(false);
              }}
            >
              {t("delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={disableOpen} onOpenChange={setDisableOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsDisableTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsDisableDescription", { skill: selected?.name || "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={disableMutation.isPending}
              onClick={() => {
                if (selected) {
                  disableMutation.mutate(selected.id, {
                    onSuccess: () => {
                      setDisableOpen(false);
                      setSelectedId(null);
                      setRightView("overview");
                      toast.success(
                        t("skillsDisabled", { skill: selected.name }),
                      );
                    },
                  });
                }
              }}
            >
              {disableMutation.isPending ? (
                <Spinner data-icon="inline-start" />
              ) : null}
              {t("skillsDisable")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={removeSymlinksOpen}
        onOpenChange={setRemoveSymlinksOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skillsRemoveSymlinksTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("skillsRemoveSymlinksDescription", {
                skill: selected?.name || "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={removeSymlinksMutation.isPending}
              onClick={() => {
                if (selected) {
                  removeSymlinksMutation.mutate(selected.id, {
                    onSuccess: () => {
                      setRemoveSymlinksOpen(false);
                      toast.success(
                        t("skillsRemoveSymlinksDone", { skill: selected.name }),
                      );
                    },
                  });
                }
              }}
            >
              {removeSymlinksMutation.isPending ? (
                <Spinner data-icon="inline-start" />
              ) : null}
              {t("skillsRemoveSymlinks")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <SkillDisabledDialog
        open={disabledListOpen}
        onOpenChange={setDisabledListOpen}
      />
    </>
  );
}
