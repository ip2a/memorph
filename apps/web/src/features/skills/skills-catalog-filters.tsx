import { useEffect, useState } from "react";
import { SlidersHorizontalIcon } from "lucide-react";
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
import { useI18n } from "@/lib/i18n-context";

export type SkillsCatalogSort = "name" | "size" | "files" | "updated";
export type SkillsCatalogOrder = "asc" | "desc";
export type SkillsCatalogScope = "all" | "global" | "project";

export type SkillsCatalogFilters = {
  provider: string;
  scope: SkillsCatalogScope;
  sort: SkillsCatalogSort;
  order: SkillsCatalogOrder;
};

const DEFAULT_FILTERS: SkillsCatalogFilters = {
  provider: "all",
  scope: "all",
  sort: "name",
  order: "asc",
};

function activeFilterCount(filters: SkillsCatalogFilters) {
  let count = 0;
  if (filters.provider !== DEFAULT_FILTERS.provider) count += 1;
  if (filters.scope !== DEFAULT_FILTERS.scope) count += 1;
  if (filters.sort !== DEFAULT_FILTERS.sort) count += 1;
  if (filters.order !== DEFAULT_FILTERS.order) count += 1;
  return count;
}

const selectClassName =
  "border-input bg-background h-9 w-full rounded-md border px-2 text-sm";

type SkillsCatalogFiltersDialogProps = {
  open: boolean;
  filters: SkillsCatalogFilters;
  providers: string[];
  onOpenChange: (open: boolean) => void;
  onApply: (next: SkillsCatalogFilters) => void;
};

export function SkillsCatalogFiltersDialog({
  open,
  filters,
  providers,
  onOpenChange,
  onApply,
}: SkillsCatalogFiltersDialogProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(filters);

  useEffect(() => {
    if (!open) return;
    setDraft(filters);
  }, [filters, open]);

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
            <Label htmlFor="skills-filter-provider">Provider</Label>
            <select
              id="skills-filter-provider"
              aria-label="Provider"
              value={draft.provider}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  provider: event.target.value,
                }))
              }
              className={selectClassName}
            >
              <option value="all">{t("skillsAllProviders")}</option>
              {providers.map((value) => (
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
              value={draft.scope}
              onChange={(event) =>
                setDraft((current) => ({
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
                value={draft.sort}
                onChange={(event) =>
                  setDraft((current) => ({
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
                value={draft.order}
                onChange={(event) =>
                  setDraft((current) => ({
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
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            type="button"
            variant="ghost"
            onClick={() => setDraft(DEFAULT_FILTERS)}
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
                onApply(draft);
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

export function SkillsCatalogFilterTrigger({
  total,
  filters,
  providers,
  onApply,
}: {
  total: number;
  filters: SkillsCatalogFilters;
  providers: string[];
  onApply: (next: SkillsCatalogFilters) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const active = activeFilterCount(filters);

  return (
    <>
      <div className="flex items-center justify-between gap-2">
        <span className="text-muted-foreground text-xs">
          {t("skillsLogicalCount", { count: total })}
        </span>
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
        providers={providers}
        onApply={onApply}
      />
    </>
  );
}
