import { useEffect, useRef, useState } from "react";
import { RefreshCwIcon, SearchIcon } from "lucide-react";
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
} from "@/features/skills/queries";
import { SkillDetailPanel } from "@/features/skills/skill-detail-panel";
import {
  SkillsCatalogFilterTrigger,
  type SkillsCatalogFilters,
} from "@/features/skills/skills-catalog-filters";
import { SkillOverviewPanel } from "@/features/skills/skill-overview-panel";
import { useI18n } from "@/lib/i18n-context";
import { useUiStore } from "@/stores/ui-store";

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
  const [rightView, setRightView] = useState<"overview" | "detail">("overview");
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [sourceProvider, setSourceProvider] = useState<string>();
  const [removalProvider, setRemovalProvider] = useState<string | null>(null);
  const initialScanStarted = useRef(false);
  const skillsQuery = useSkills({
    query: search.trim() || undefined,
    provider: provider === "all" ? undefined : provider,
    scope: scope === "all" ? undefined : (scope as "global" | "project"),
    sort,
    order,
    page,
    pageSize: 50,
  });
  const currentWorkspace =
    useUiStore((state) => state.selectedWorkspace) ?? undefined;
  const scanMutation = useScanSkills();
  const installMutation = useInstallSkill();
  const uninstallMutation = useUninstallSkill();
  const items = skillsQuery.data?.items ?? [];
  const selected = items.find((item) => item.id === selectedId) ?? null;
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

  useEffect(() => {
    if (
      initialScanStarted.current ||
      skillsQuery.data?.completeness.status !== "unknown"
    )
      return;
    initialScanStarted.current = true;
    scanMutation.mutate({
      mode: "incremental",
      workspace: currentWorkspace,
    });
  }, [currentWorkspace, scanMutation, skillsQuery.data?.completeness.status]);

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
  const catalogFilters: SkillsCatalogFilters = {
    provider,
    scope: scope as SkillsCatalogFilters["scope"],
    sort,
    order,
  };

  function applyCatalogFilters(next: SkillsCatalogFilters) {
    setProvider(next.provider);
    setScope(next.scope);
    setSort(next.sort);
    setOrder(next.order);
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
                    onClick={() =>
                      scanMutation.mutate(
                        { mode: "incremental", workspace: currentWorkspace },
                        {
                          onSuccess: (result) =>
                            toast.success(
                              t("skillsScanComplete", {
                                skills: result.skills_seen,
                                installations: result.installations_seen,
                              }),
                            ),
                        },
                      )
                    }
                    disabled={scanMutation.isPending}
                  >
                    {scanMutation.isPending ? <Spinner /> : <RefreshCwIcon />}
                    {t("skillsIncrementalScan")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      if (window.confirm(t("skillsFullScanConfirm"))) {
                        scanMutation.mutate({
                          mode: "full",
                          workspace: currentWorkspace,
                        });
                      }
                    }}
                    disabled={scanMutation.isPending}
                  >
                    {t("skillsFullScan")}
                  </Button>
                </div>
              </div>
              {skillsQuery.data?.completeness.status !== "complete" ? (
                <div className="rounded-md border border-amber-500/50 bg-amber-500/10 px-3 py-2 text-xs">
                  {t("skillsIndexingHint")}
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
              <SkillsCatalogFilterTrigger
                total={skillsQuery.data?.total ?? 0}
                filters={catalogFilters}
                providers={skillsQuery.data?.providers ?? []}
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
                    meta={item.description || item.source_id}
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
            <div className="flex items-center justify-between border-t pt-3 text-xs">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((value) => value - 1)}
              >
                {t("skillsPreviousPage")}
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
                {t("skillsNextPage")}
              </Button>
            </div>
          </PanelCard>

          <PanelCard className="flex min-h-0 flex-col gap-4 p-4">
            {rightView === "overview" ? (
              <SkillOverviewPanel
                skillId={selectedId}
                provider={provider === "all" ? undefined : provider}
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
                sourceProvider={selectedSource}
                onSourceProviderChange={setSourceProvider}
                pending={pending}
                mutationError={mutationError}
                provider={provider === "all" ? undefined : provider}
                onInstall={(agent) =>
                  selected &&
                  installMutation.mutate({
                    skill_id: selected.source_id,
                    provider: agent,
                    source_provider: selectedSource,
                  })
                }
                onRemove={setRemovalProvider}
              />
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
