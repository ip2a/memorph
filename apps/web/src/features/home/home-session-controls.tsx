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
import { ProviderPicker } from "@/features/home/provider-picker";
import type { ProviderCatalogEntry, SessionHookFilter, SessionListSort } from "@/lib/types";
import { cn } from "@/lib/utils";

const SORT_OPTIONS: Array<{ label: string; value: SessionListSort }> = [
  { label: "Recent", value: "recent" },
  { label: "Title", value: "title" },
  { label: "Hook attention", value: "hook_attention" },
];

const HOOK_FILTER_OPTIONS: Array<{ label: string; value: SessionHookFilter }> = [
  { label: "All hooks", value: "all" },
  { label: "Attention", value: "attention" },
  { label: "Runtime", value: "runtime" },
  { label: "Linked", value: "linked" },
  { label: "No hook", value: "no_hook" },
];

type HomeSortDialogProps = {
  open: boolean;
  sort: SessionListSort;
  onOpenChange: (open: boolean) => void;
  onSortChange: (sort: SessionListSort) => void;
};

export function HomeSortDialog({ open, sort, onOpenChange, onSortChange }: HomeSortDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-home-sort-dialog>
        <DialogHeader>
          <DialogTitle>Sort sessions</DialogTitle>
          <DialogDescription>Choose how recent sessions are ordered in the list below.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          {SORT_OPTIONS.map((option) => (
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
              {option.label}
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
  hookFilter: SessionHookFilter;
  selectedProviders: string[];
  sessionsPerProvider: number;
  defaultSessionsPerProvider: number;
  providerCandidates: ProviderCatalogEntry[];
  onOpenChange: (open: boolean) => void;
  onApply: (next: { hookFilter: SessionHookFilter; selectedProviders: string[]; sessionsPerProvider: number }) => void;
};

export function HomeFiltersDialog({
  open,
  hookFilter,
  selectedProviders,
  sessionsPerProvider,
  defaultSessionsPerProvider,
  providerCandidates,
  onOpenChange,
  onApply,
}: HomeFiltersDialogProps) {
  const [draftHookFilter, setDraftHookFilter] = useState(hookFilter);
  const [draftProviders, setDraftProviders] = useState(selectedProviders);
  const [draftSessionsPerProvider, setDraftSessionsPerProvider] = useState(sessionsPerProvider);

  useEffect(() => {
    if (!open) return;
    // Reset draft state to the current applied filters whenever the dialog opens.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setDraftHookFilter(hookFilter);
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setDraftProviders(selectedProviders);
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setDraftSessionsPerProvider(sessionsPerProvider);
  }, [hookFilter, open, selectedProviders, sessionsPerProvider]);

  function toggleDraftProvider(providerId: string) {
    setDraftProviders((current) =>
      current.includes(providerId) ? current.filter((id) => id !== providerId) : [...current, providerId],
    );
  }

  function resetDraft() {
    setDraftHookFilter("all");
    setDraftProviders(providerCandidates.map((item) => item.provider_id));
    setDraftSessionsPerProvider(defaultSessionsPerProvider);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg" data-home-filters-dialog>
        <DialogHeader>
          <DialogTitle>Filter sessions</DialogTitle>
          <DialogDescription>Select scan providers and hook visibility for the recent session list.</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5">
          <section className="grid gap-2">
            <p className="font-mono text-xs uppercase text-muted-foreground">Providers</p>
            <ProviderPicker candidates={providerCandidates} selected={draftProviders} onToggle={toggleDraftProvider} />
          </section>

          <section className="grid gap-2">
            <p className="font-mono text-xs uppercase text-muted-foreground">Sessions per agent</p>
            <p className="text-sm text-muted-foreground">How many recent sessions to show for each agent on the home page.</p>
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
                aria-label="Sessions per agent"
              />
            </div>
          </section>

          <section className="grid gap-2">
            <p className="font-mono text-xs uppercase text-muted-foreground">Hook status</p>
            <div className="grid gap-2 sm:grid-cols-2">
              {HOOK_FILTER_OPTIONS.map((option) => (
                <Button
                  key={option.value}
                  type="button"
                  variant={draftHookFilter === option.value ? "secondary" : "outline"}
                  className="justify-start"
                  onClick={() => setDraftHookFilter(option.value)}
                >
                  {option.label}
                </Button>
              ))}
            </div>
          </section>
        </div>

        <DialogFooter className="gap-2 sm:justify-between">
          <Button type="button" variant="ghost" onClick={resetDraft}>
            Reset
          </Button>
          <div className="flex gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!draftProviders.length}
              onClick={() => {
                onApply({
                  hookFilter: draftHookFilter,
                  selectedProviders: draftProviders,
                  sessionsPerProvider: draftSessionsPerProvider,
                });
                onOpenChange(false);
              }}
            >
              Apply
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
  hookFilter: SessionHookFilter;
  selectedProviders: string[];
  sessionsPerProvider: number;
  defaultSessionsPerProvider: number;
  onSearchChange: (value: string) => void;
  onSortChange: (sort: SessionListSort) => void;
  onFiltersApply: (next: {
    hookFilter: SessionHookFilter;
    selectedProviders: string[];
    sessionsPerProvider: number;
  }) => void;
  providerCandidates: ProviderCatalogEntry[];
};

function activeFilterCount(
  hookFilter: SessionHookFilter,
  selectedProviders: string[],
  providerCandidates: ProviderCatalogEntry[],
  sessionsPerProvider: number,
  defaultSessionsPerProvider: number,
) {
  let count = 0;
  if (hookFilter !== "all") count += 1;
  if (selectedProviders.length !== providerCandidates.length) count += 1;
  if (sessionsPerProvider !== defaultSessionsPerProvider) count += 1;
  return count;
}

export function HomeSessionToolbar({
  className,
  search,
  sort,
  hookFilter,
  selectedProviders,
  sessionsPerProvider,
  defaultSessionsPerProvider,
  onSearchChange,
  onSortChange,
  onFiltersApply,
  providerCandidates,
}: HomeSessionToolbarProps) {
  const [sortOpen, setSortOpen] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filtersActive = activeFilterCount(
    hookFilter,
    selectedProviders,
    providerCandidates,
    sessionsPerProvider,
    defaultSessionsPerProvider,
  );
  const sortLabel = SORT_OPTIONS.find((option) => option.value === sort)?.label ?? "Sort";

  return (
    <>
      <div className={cn("flex min-w-0 flex-1 items-center gap-3", className)}>
        <div className="relative min-w-0 flex-[0_1_80%]">
          <SearchIcon className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            className="w-full pl-8"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder="Search sessions"
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
          {sort === "recent" ? "Sort" : sortLabel}
        </Button>

        <Button
          type="button"
          variant={filtersActive ? "secondary" : "outline"}
          onClick={() => setFiltersOpen(true)}
          data-home-filters-trigger
        >
          <SlidersHorizontalIcon data-icon="inline-start" />
          Filters
          {filtersActive ? <span className="font-mono text-xs">({filtersActive})</span> : null}
        </Button>
        </div>
      </div>

      <HomeSortDialog open={sortOpen} onOpenChange={setSortOpen} sort={sort} onSortChange={onSortChange} />
      <HomeFiltersDialog
        open={filtersOpen}
        onOpenChange={setFiltersOpen}
        hookFilter={hookFilter}
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
  if (errorMessage) {
    return (
      <div className="px-3 pb-3">
        <div className="rounded-md border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive">{errorMessage}</div>
      </div>
    );
  }

  return (
    <div className="relative min-h-48 px-3 pb-3">
      {loading ? <SessionListLoadingOverlay label="Loading sessions..." /> : null}
      {!loading && refreshing ? <SessionListLoadingOverlay label="Updating sessions..." /> : null}
      <div className={cn((loading || refreshing) && "pointer-events-none opacity-60")}>{children}</div>
    </div>
  );
}
