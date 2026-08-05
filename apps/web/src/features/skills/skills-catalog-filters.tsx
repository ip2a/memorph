import { useEffect, useState } from "react";
import { ChevronLeftIcon, ChevronRightIcon, SlidersHorizontalIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { SkillsCatalogPageSizeControls } from "@/features/skills/skills-catalog-page-size-controls";
import {
  clampSkillsCatalogPageSize,
  DEFAULT_SKILLS_CATALOG_PAGE_SIZE,
} from "@/features/skills/skills-catalog-page-size";
import { useI18n } from "@/lib/i18n-context";

export type SkillsCatalogSort = "name" | "size" | "files" | "updated";
export type SkillsCatalogOrder = "asc" | "desc";
export type SkillsCatalogScope = "all" | "global" | "project";

export type SkillsCatalogFilters = {
  used_by: string;
  scope: SkillsCatalogScope;
  sort: SkillsCatalogSort;
  order: SkillsCatalogOrder;
};

const DEFAULT_FILTERS: SkillsCatalogFilters = {
  used_by: "all",
  scope: "all",
  sort: "name",
  order: "asc",
};

function activeFilterCount(filters: SkillsCatalogFilters, pageSize: number) {
  let count = 0;
  if (filters.used_by !== DEFAULT_FILTERS.used_by) count += 1;
  if (filters.scope !== DEFAULT_FILTERS.scope) count += 1;
  if (filters.sort !== DEFAULT_FILTERS.sort) count += 1;
  if (filters.order !== DEFAULT_FILTERS.order) count += 1;
  if (pageSize !== DEFAULT_SKILLS_CATALOG_PAGE_SIZE) count += 1;
  return count;
}

export type SkillsCatalogFilterApply = {
  filters: SkillsCatalogFilters;
  page_size: number;
};

const selectClassName =
  "border-input bg-background h-9 w-full rounded-md border px-2 text-sm";

type SkillsCatalogFiltersDialogProps = {
  open: boolean;
  filters: SkillsCatalogFilters;
  pageSize: number;
  usedBy: string[];
  onOpenChange: (open: boolean) => void;
  onApply: (next: SkillsCatalogFilterApply) => void;
};

export function SkillsCatalogFiltersDialog({
  open,
  filters,
  pageSize,
  usedBy,
  onOpenChange,
  onApply,
}: SkillsCatalogFiltersDialogProps) {
  const { t } = useI18n();
  const [draftFilters, setDraftFilters] = useState(filters);
  const [draftPageSize, setDraftPageSize] = useState(pageSize);

  useEffect(() => {
    if (!open) return;
    setDraftFilters(filters);
    setDraftPageSize(pageSize);
  }, [filters, open, pageSize]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-skills-filters-dialog>
        <DialogHeader>
          <DialogTitle>{t("skillsFilterTitle")}</DialogTitle>
          <DialogDescription>
            {t("skillsFilterDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-2">
            <Label htmlFor="skills-filter-used-by">{t("skillsUsedBy")}</Label>
            <select
              id="skills-filter-used-by"
              aria-label={t("skillsUsedBy")}
              value={draftFilters.used_by}
              onChange={(event) =>
                setDraftFilters((current) => ({
                  ...current,
                  used_by: event.target.value,
                }))
              }
              className={selectClassName}
            >
              <option value="all">{t("skillsAllUsedBy")}</option>
              {usedBy.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="skills-filter-scope">{t("skillsInstallScope")}</Label>
            <select
              id="skills-filter-scope"
              aria-label={t("skillsInstallScope")}
              value={draftFilters.scope}
              onChange={(event) =>
                setDraftFilters((current) => ({
                  ...current,
                  scope: event.target.value as SkillsCatalogScope,
                }))
              }
              className={selectClassName}
            >
              <option value="all">{t("skillsAllScopes")}</option>
              <option value="global">{t("skillsGlobalScope")}</option>
              <option value="project">{t("skillsProjectScopeOption")}</option>
            </select>
          </div>
          <div className="grid gap-2 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label htmlFor="skills-filter-sort">{t("skillsSortField")}</Label>
              <select
                id="skills-filter-sort"
                aria-label={t("skillsSortField")}
                value={draftFilters.sort}
                onChange={(event) =>
                  setDraftFilters((current) => ({
                    ...current,
                    sort: event.target.value as SkillsCatalogSort,
                  }))
                }
                className={selectClassName}
              >
                <option value="name">{t("skillsName")}</option>
                <option value="size">{t("skillsSize")}</option>
                <option value="files">{t("skillsFileCount")}</option>
                <option value="updated">{t("skillsUpdatedAt")}</option>
              </select>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="skills-filter-order">{t("skillsSortOrder")}</Label>
              <select
                id="skills-filter-order"
                aria-label={t("skillsSortOrder")}
                value={draftFilters.order}
                onChange={(event) =>
                  setDraftFilters((current) => ({
                    ...current,
                    order: event.target.value as SkillsCatalogOrder,
                  }))
                }
                className={selectClassName}
              >
                <option value="asc">{t("skillsAscending")}</option>
                <option value="desc">{t("skillsDescending")}</option>
              </select>
            </div>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="skills-filter-page-size">
              {t("skillsCatalogPageSizePreference")}
            </Label>
            <SkillsCatalogPageSizeControls
              value={draftPageSize}
              onChange={setDraftPageSize}
            />
            <p className="text-muted-foreground text-xs">
              {t("skillsCatalogPageSizePreferenceHint")}
            </p>
          </div>
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              setDraftFilters(DEFAULT_FILTERS);
              setDraftPageSize(DEFAULT_SKILLS_CATALOG_PAGE_SIZE);
            }}
          >
            {t("skillsReset")}
          </Button>
          <div className="flex gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              {t("cancel")}
            </Button>
            <Button
              type="button"
              onClick={() => {
                onApply({
                  filters: draftFilters,
                  page_size: clampSkillsCatalogPageSize(draftPageSize),
                });
                onOpenChange(false);
              }}
            >
              {t("skillsApply")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export type SkillsCatalogPagination = {
  rangeFrom: number;
  rangeTo: number;
  total: number;
  page: number;
  pageCount: number;
  onPrevious: () => void;
  onNext: () => void;
};

function formatPaginationSummary({
  rangeFrom,
  rangeTo,
  total,
  page,
  pageCount,
}: SkillsCatalogPagination) {
  if (total <= 0) return null;
  const range =
    rangeFrom === rangeTo ? String(rangeFrom) : `${rangeFrom}–${rangeTo}`;
  const slice = `${range}/${total}`;
  if (pageCount > 1) return `${slice} · ${page}/${pageCount}`;
  return slice;
}

export function SkillsCatalogFilterTrigger({
  pagination,
  filters,
  pageSize,
  usedBy,
  onApply,
}: {
  pagination: SkillsCatalogPagination;
  filters: SkillsCatalogFilters;
  pageSize: number;
  usedBy: string[];
  onApply: (next: SkillsCatalogFilterApply) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const active = activeFilterCount(filters, pageSize);
  const summary = formatPaginationSummary(pagination);
  const { page, pageCount, onPrevious, onNext } = pagination;

  return (
    <>
      <div className="flex items-center justify-between gap-2">
        {summary ? (
          <div className="flex shrink-0 items-center gap-0.5">
            {pageCount > 1 ? (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t("skillsPreviousPage")}
                disabled={page <= 1}
                onClick={onPrevious}
              >
                <ChevronLeftIcon />
              </Button>
            ) : null}
            <span className="text-muted-foreground font-mono text-xs tabular-nums whitespace-nowrap">
              {summary}
            </span>
            {pageCount > 1 ? (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t("skillsNextPage")}
                disabled={page >= pageCount}
                onClick={onNext}
              >
                <ChevronRightIcon />
              </Button>
            ) : null}
          </div>
        ) : (
          <span />
        )}
        <Button
          type="button"
          variant={active ? "secondary" : "outline"}
          size="sm"
          onClick={() => setOpen(true)}
          data-skills-filters-trigger
        >
          <SlidersHorizontalIcon data-icon="inline-start" />
          {t("skillsFilter")}
          {active ? (
            <span className="font-mono text-xs">({active})</span>
          ) : null}
        </Button>
      </div>
      <SkillsCatalogFiltersDialog
        open={open}
        onOpenChange={setOpen}
        filters={filters}
        pageSize={pageSize}
        usedBy={usedBy}
        onApply={onApply}
      />
    </>
  );
}
