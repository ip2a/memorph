import { useEffect, useState, type ReactNode } from "react";
import { ArrowDownUpIcon, SearchIcon, SlidersHorizontalIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { orderProviderPills } from "@/features/home/model/providers";
import { ProviderPicker } from "@/features/home/provider-picker";
import { useI18n } from "@/lib/i18n-context";
import type { I18nKey } from "@/lib/i18n-core";
import type { ProviderCatalogEntry, SessionListSort } from "@/lib/types";
import { cn } from "@/lib/utils";

const SORT_OPTION_KEYS: Array<{ labelKey: I18nKey; value: SessionListSort }> = [
  { labelKey: "sortByRecent", value: "recent" },
  { labelKey: "sortByTitle", value: "title" },
];

type HomeSortDialogProps = {
  open: boolean;
  sort: SessionListSort;
  onOpenChange: (open: boolean) => void;
  onSortChange: (sort: SessionListSort) => void;
};

export function HomeSortDialog({ open, sort, onOpenChange, onSortChange }: HomeSortDialogProps) {
  const { t } = useI18n();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-home-sort-dialog>
        <DialogHeader>
          <DialogTitle>{t("sortSessions")}</DialogTitle>
          <DialogDescription>{t("sortSessionsDescription")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          {SORT_OPTION_KEYS.map((option) => (
            <Button
              key={option.value}
              type="button"
              variant={sort === option.value ? "secondary" : "outline"}
              className="justify-start"
              onClick={() => {
                onSortChange(option.value);
                onOpenChange(false);
              }}
            >
              {t(option.labelKey)}
            </Button>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}

const SESSIONS_PER_AGENT_PRESETS = [6, 12, 24, 50] as const;

type HomeFiltersDialogProps = {
  open: boolean;
  selectedProviders: string[];
  sessionsPerProvider: number;
  defaultSessionsPerProvider: number;
  providerCandidates: ProviderCatalogEntry[];
  onOpenChange: (open: boolean) => void;
  onApply: (next: { selectedProviders: string[]; sessionsPerProvider: number }) => void;
};

export function HomeFiltersDialog({
  open,
  selectedProviders,
  sessionsPerProvider,
  defaultSessionsPerProvider,
  providerCandidates,
  onOpenChange,
  onApply,
}: HomeFiltersDialogProps) {
  const { t } = useI18n();
  const [draftProviders, setDraftProviders] = useState(selectedProviders);
  const [draftSessionsPerProvider, setDraftSessionsPerProvider] = useState(sessionsPerProvider);
  const [providerOrder, setProviderOrder] = useState(providerCandidates);

  useEffect(() => {
    if (!open) return;
    setDraftProviders(selectedProviders);
    setDraftSessionsPerProvider(sessionsPerProvider);
    setProviderOrder(orderProviderPills(providerCandidates, selectedProviders));
  }, [open, providerCandidates, selectedProviders, sessionsPerProvider]);

  function toggleDraftProvider(providerId: string) {
    setDraftProviders((current) =>
      current.includes(providerId) ? current.filter((id) => id !== providerId) : [...current, providerId],
    );
  }

  function resetDraft() {
    const allProviderIds = providerCandidates.map((item) => item.provider_id);
    setDraftProviders(allProviderIds);
    setDraftSessionsPerProvider(defaultSessionsPerProvider);
    setProviderOrder(orderProviderPills(providerCandidates, allProviderIds));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg" data-home-filters-dialog>
        <DialogHeader>
          <DialogTitle>{t("filterSessions")}</DialogTitle>
          <DialogDescription>{t("filterSessionsDescription")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5">
          <section className="grid gap-2">
            <p className="font-mono text-xs uppercase text-muted-foreground">{t("providers")}</p>
            <ProviderPicker candidates={providerOrder} selected={draftProviders} onToggle={toggleDraftProvider} />
          </section>

          <section className="grid gap-2">
            <p className="font-mono text-xs uppercase text-muted-foreground">{t("sessionsPerProvider")}</p>
            <p className="text-sm text-muted-foreground">{t("sessionsPerProviderHint")}</p>
            <div className="flex flex-wrap items-center gap-2">
              {SESSIONS_PER_AGENT_PRESETS.map((value) => (
                <Button
                  key={value}
                  type="button"
                  variant={draftSessionsPerProvider === value ? "secondary" : "outline"}
                  className="min-w-12"
                  onClick={() => setDraftSessionsPerProvider(value)}
                >
                  {value}
                </Button>
              ))}
              <Input
                className="w-20"
                type="number"
                min={1}
                max={200}
                value={draftSessionsPerProvider}
                onChange={(event) => setDraftSessionsPerProvider(Math.max(1, Math.min(200, Number(event.target.value || 1))))}
                aria-label={t("sessionsPerProvider")}
              />
            </div>
          </section>
        </div>

        <DialogFooter className="gap-2 sm:justify-between">
          <Button type="button" variant="ghost" onClick={resetDraft}>
            {t("reset")}
          </Button>
          <div className="flex gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t("cancel")}
            </Button>
            <Button
              type="button"
              disabled={!draftProviders.length}
              onClick={() => {
                onApply({
                  selectedProviders: draftProviders,
                  sessionsPerProvider: draftSessionsPerProvider,
                });
                onOpenChange(false);
              }}
            >
              {t("apply")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type HomeSessionToolbarProps = {
  className?: string;
  search: string;
  sort: SessionListSort;
  selectedProviders: string[];
  sessionsPerProvider: number;
  defaultSessionsPerProvider: number;
  onSearchChange: (value: string) => void;
  onSortChange: (sort: SessionListSort) => void;
  onFiltersApply: (next: {
    selectedProviders: string[];
    sessionsPerProvider: number;
  }) => void;
  providerCandidates: ProviderCatalogEntry[];
};

function activeFilterCount(
  selectedProviders: string[],
  providerCandidates: ProviderCatalogEntry[],
  sessionsPerProvider: number,
  defaultSessionsPerProvider: number,
) {
  let count = 0;
  if (selectedProviders.length !== providerCandidates.length) count += 1;
  if (sessionsPerProvider !== defaultSessionsPerProvider) count += 1;
  return count;
}

export function HomeSessionToolbar({
  className,
  search,
  sort,
  selectedProviders,
  sessionsPerProvider,
  defaultSessionsPerProvider,
  onSearchChange,
  onSortChange,
  onFiltersApply,
  providerCandidates,
}: HomeSessionToolbarProps) {
  const { t } = useI18n();
  const [sortOpen, setSortOpen] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filtersActive = activeFilterCount(
    selectedProviders,
    providerCandidates,
    sessionsPerProvider,
    defaultSessionsPerProvider,
  );
  const sortOption = SORT_OPTION_KEYS.find((option) => option.value === sort);
  const sortLabel = sortOption ? t(sortOption.labelKey) : t("sort");

  return (
    <>
      <div className={cn("flex min-w-0 flex-1 items-center gap-3", className)}>
        <div className="relative min-w-0 flex-[0_1_80%]">
          <SearchIcon className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            className="w-full pl-8"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={t("searchSessions")}
          />
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
        <Button
          type="button"
          variant={sort === "recent" ? "outline" : "secondary"}
          onClick={() => setSortOpen(true)}
          data-home-sort-trigger
        >
          <ArrowDownUpIcon data-icon="inline-start" />
          {sort === "recent" ? t("sort") : sortLabel}
        </Button>

        <Button
          type="button"
          variant={filtersActive ? "secondary" : "outline"}
          onClick={() => setFiltersOpen(true)}
          data-home-filters-trigger
        >
          <SlidersHorizontalIcon data-icon="inline-start" />
          {t("filters")}
          {filtersActive ? <span className="font-mono text-xs">({filtersActive})</span> : null}
        </Button>
        </div>
      </div>

      <HomeSortDialog open={sortOpen} onOpenChange={setSortOpen} sort={sort} onSortChange={onSortChange} />
      <HomeFiltersDialog
        open={filtersOpen}
        onOpenChange={setFiltersOpen}
        selectedProviders={selectedProviders}
        sessionsPerProvider={sessionsPerProvider}
        defaultSessionsPerProvider={defaultSessionsPerProvider}
        providerCandidates={providerCandidates}
        onApply={onFiltersApply}
      />
    </>
  );
}

function SessionListLoadingOverlay({ label }: { label: string }) {
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center rounded-md bg-background/75 backdrop-blur-[1px]">
      <div className="flex items-center gap-2 font-mono text-sm text-muted-foreground">
        <span className="size-4 animate-spin rounded-full border-2 border-current border-t-transparent" aria-hidden />
        {label}
      </div>
    </div>
  );
}

export function HomeSessionListPanel({
  loading,
  refreshing,
  errorMessage,
  children,
}: {
  loading: boolean;
  refreshing: boolean;
  errorMessage?: string | null;
  children: ReactNode;
}) {
  const { t } = useI18n();

  if (errorMessage) {
    return (
      <div className="px-3 pb-3">
        <div className="rounded-md border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive">{errorMessage}</div>
      </div>
    );
  }

  return (
    <div className="relative flex min-h-full flex-col px-3 pb-3">
      {loading ? <SessionListLoadingOverlay label={t("loadingSessions")} /> : null}
      {!loading && refreshing ? <SessionListLoadingOverlay label={t("updatingSessions")} /> : null}
      <div className={cn("flex min-h-0 flex-1 flex-col", (loading || refreshing) && "pointer-events-none opacity-60")}>
        {children}
      </div>
    </div>
  );
}
