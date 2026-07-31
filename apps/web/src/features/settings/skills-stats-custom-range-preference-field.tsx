import { useEffect, useState } from "react";
import { Field, FieldContent, FieldDescription, FieldTitle } from "@/components/ui/field";
import {
  countSkillsStatsRangeDays,
  formatSkillsStatsRangeLabel,
  readSkillsStatsCustomRange,
  writeSkillsStatsCustomRange,
} from "@/features/skills/skills-stats-preferences";
import { useI18n } from "@/lib/i18n-context";

function defaultCustomDates() {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - 29);
  const localDate = (date: Date) => {
    const offset = date.getTimezoneOffset() * 60_000;
    return new Date(date.getTime() - offset).toISOString().slice(0, 10);
  };
  return { from: localDate(from), to: localDate(to) };
}

export function SkillsStatsCustomRangePreferenceField() {
  const { t } = useI18n();
  const [range, setRange] = useState(defaultCustomDates);

  useEffect(() => {
    setRange(readSkillsStatsCustomRange() ?? defaultCustomDates());
  }, []);

  function update(next: Partial<{ from: string; to: string }>) {
    setRange((current) => {
      const value = { ...current, ...next };
      writeSkillsStatsCustomRange(value);
      return value;
    });
  }

  const dayCount = countSkillsStatsRangeDays(range.from, range.to);
  const rangeLabel = formatSkillsStatsRangeLabel(range.from, range.to);

  return (
    <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start">
      <Field orientation="vertical">
        <FieldContent>
          <FieldTitle>{t("skillsCustomRangePreference")}</FieldTitle>
          <FieldDescription>{t("skillsCustomRangePreferenceHint")}</FieldDescription>
        </FieldContent>
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <input
            aria-label={t("skillsStartDate")}
            className="border-input bg-background h-9 rounded-md border px-2"
            type="date"
            value={range.from}
            onChange={(event) => update({ from: event.target.value })}
          />
          <span>{t("skillsTo")}</span>
          <input
            aria-label={t("skillsEndDate")}
            className="border-input bg-background h-9 rounded-md border px-2"
            type="date"
            value={range.to}
            onChange={(event) => update({ to: event.target.value })}
          />
        </div>
      </Field>

      <div
        className="flex min-w-[8.5rem] flex-col gap-1 rounded-md border bg-muted/30 px-3 py-2.5 text-sm sm:text-right"
        aria-live="polite"
      >
        <span className="text-xs text-muted-foreground">{t("skillsCustomRangePreview")}</span>
        <span className="font-medium tabular-nums">{rangeLabel}</span>
        <span className="text-muted-foreground tabular-nums">
          {dayCount > 0 ? t("skillsDays", { count: dayCount }) : t("skillsCustomRangeInvalid")}
        </span>
      </div>
    </div>
  );
}
