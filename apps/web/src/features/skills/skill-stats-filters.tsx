import { useSearchParams } from "react-router-dom";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
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

export function useSkillStatsFilters(provider?: string) {
  const [searchParams, setSearchParams] = useSearchParams();
  const rawRange = searchParams.get("statsRange");
  const range: SkillStatsRange =
    rawRange === "custom" || (rawRange && rawRange in RANGE_DAYS)
      ? (rawRange as SkillStatsRange)
      : "30d";
  const dates =
    range === "custom"
      ? {
          from: searchParams.get("statsFrom") || localDate(new Date()),
          to: searchParams.get("statsTo") || localDate(new Date()),
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

export function SkillStatsFilterTabs({ className }: { className?: string }) {
  const { t } = useI18n();
  const { range, update } = useSkillStatsFilters();

  return (
    <div className={className}>
      <Tabs
        value={range}
        onValueChange={(value) =>
          update({ statsRange: value, statsPage: null })
        }
      >
        <TabsList aria-label={t("skillsStatsRange")}>
          <TabsTrigger value="7d">{t("skillsDays", { count: 7 })}</TabsTrigger>
          <TabsTrigger value="30d">{t("skillsDays", { count: 30 })}</TabsTrigger>
          <TabsTrigger value="90d">{t("skillsDays", { count: 90 })}</TabsTrigger>
          <TabsTrigger value="custom">{t("skillsCustomRange")}</TabsTrigger>
        </TabsList>
      </Tabs>
    </div>
  );
}

export function SkillStatsCustomDateRange({ className }: { className?: string }) {
  const { t } = useI18n();
  const { range, dates, update } = useSkillStatsFilters();
  if (range !== "custom") return null;

  return (
    <div
      className={`flex flex-wrap items-center justify-end gap-2 text-sm ${className ?? ""}`}
    >
      <input
        aria-label={t("skillsStartDate")}
        className="border-input bg-background h-8 rounded-md border px-2"
        type="date"
        value={dates.from}
        onChange={(event) =>
          update({ statsFrom: event.target.value, statsPage: null })
        }
      />
      <span>{t("skillsTo")}</span>
      <input
        aria-label={t("skillsEndDate")}
        className="border-input bg-background h-8 rounded-md border px-2"
        type="date"
        value={dates.to}
        onChange={(event) =>
          update({ statsTo: event.target.value, statsPage: null })
        }
      />
    </div>
  );
}
