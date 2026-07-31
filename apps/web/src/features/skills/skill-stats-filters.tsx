import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  formatSkillsStatsRangeLabel,
  readSkillsStatsCustomRange,
  writeSkillsStatsCustomRange,
} from "@/features/skills/skills-stats-preferences";
import type { SkillStatsParams } from "@/lib/types";
import { useI18n } from "@/lib/i18n-context";

const RANGE_DAYS = { "7d": 7, "30d": 30, "90d": 90 } as const;
export type SkillStatsRange = keyof typeof RANGE_DAYS | "custom";

function localDate(date: Date) {
  const offset = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offset).toISOString().slice(0, 10);
}

function presetRange(range: keyof typeof RANGE_DAYS) {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - RANGE_DAYS[range] + 1);
  return { from: localDate(from), to: localDate(to) };
}

function defaultCustomDates() {
  return readSkillsStatsCustomRange() ?? presetRange("30d");
}

export function useSkillStatsFilters(provider?: string) {
  const [searchParams, setSearchParams] = useSearchParams();
  const rawRange = searchParams.get("statsRange");
  const range: SkillStatsRange =
    rawRange === "custom" || (rawRange && rawRange in RANGE_DAYS)
      ? (rawRange as SkillStatsRange)
      : "30d";
  const savedCustom = readSkillsStatsCustomRange();
  const dates =
    range === "custom"
      ? {
          from:
            searchParams.get("statsFrom") ||
            savedCustom?.from ||
            defaultCustomDates().from,
          to:
            searchParams.get("statsTo") ||
            savedCustom?.to ||
            defaultCustomDates().to,
        }
      : presetRange(range);
  const params: SkillStatsParams = {
    ...dates,
    provider,
  };

  function update(values: Record<string, string | null>) {
    const next = new URLSearchParams(searchParams);
    Object.entries(values).forEach(([key, value]) =>
      value ? next.set(key, value) : next.delete(key),
    );
    setSearchParams(next, { replace: true });
  }

  return { range, dates, params, update };
}

type SkillStatsCustomRangeDialogProps = {
  open: boolean;
  initialFrom: string;
  initialTo: string;
  onOpenChange: (open: boolean) => void;
  onApply: (range: { from: string; to: string }) => void;
};

export function SkillStatsCustomRangeDialog({
  open,
  initialFrom,
  initialTo,
  onOpenChange,
  onApply,
}: SkillStatsCustomRangeDialogProps) {
  const { t } = useI18n();
  const [from, setFrom] = useState(initialFrom);
  const [to, setTo] = useState(initialTo);

  useEffect(() => {
    if (!open) return;
    setFrom(initialFrom);
    setTo(initialTo);
  }, [initialFrom, initialTo, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" data-skills-stats-custom-range-dialog>
        <DialogHeader>
          <DialogTitle>{t("skillsCustomRangeDialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("skillsCustomRangeDialogDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <input
            aria-label={t("skillsStartDate")}
            className="border-input bg-background h-9 flex-1 rounded-md border px-2"
            type="date"
            value={from}
            onChange={(event) => setFrom(event.target.value)}
          />
          <span>{t("skillsTo")}</span>
          <input
            aria-label={t("skillsEndDate")}
            className="border-input bg-background h-9 flex-1 rounded-md border px-2"
            type="date"
            value={to}
            onChange={(event) => setTo(event.target.value)}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("cancel")}
          </Button>
          <Button
            disabled={!from || !to || from > to}
            onClick={() => {
              writeSkillsStatsCustomRange({ from, to });
              onApply({ from, to });
              onOpenChange(false);
            }}
          >
            {t("skillsApply")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SkillStatsFilterTabs({ className }: { className?: string }) {
  const { t } = useI18n();
  const { range, dates, update } = useSkillStatsFilters();
  const [customDialogOpen, setCustomDialogOpen] = useState(false);
  const [dialogDates, setDialogDates] = useState(defaultCustomDates);

  function openCustomDialog(nextDates = defaultCustomDates()) {
    setDialogDates(nextDates);
    setCustomDialogOpen(true);
  }

  return (
    <div className={className}>
      <Tabs
        value={range}
        onValueChange={(value) => {
          if (value === "custom") {
            openCustomDialog(
              range === "custom" ? dates : defaultCustomDates(),
            );
            return;
          }
          update({
            statsRange: value,
            statsFrom: null,
            statsTo: null,
            statsPage: null,
          });
        }}
      >
        <TabsList aria-label={t("skillsStatsRange")}>
          <TabsTrigger value="7d">{t("skillsDays", { count: 7 })}</TabsTrigger>
          <TabsTrigger value="30d">{t("skillsDays", { count: 30 })}</TabsTrigger>
          <TabsTrigger value="90d">{t("skillsDays", { count: 90 })}</TabsTrigger>
          <TabsTrigger
            value="custom"
            onClick={() => {
              if (range === "custom") {
                openCustomDialog(dates);
              }
            }}
          >
            {range === "custom"
              ? formatSkillsStatsRangeLabel(dates.from, dates.to)
              : t("skillsCustomRange")}
          </TabsTrigger>
        </TabsList>
      </Tabs>
      <SkillStatsCustomRangeDialog
        open={customDialogOpen}
        initialFrom={dialogDates.from}
        initialTo={dialogDates.to}
        onOpenChange={setCustomDialogOpen}
        onApply={({ from, to }) =>
          update({
            statsRange: "custom",
            statsFrom: from,
            statsTo: to,
            statsPage: null,
          })
        }
      />
    </div>
  );
}
