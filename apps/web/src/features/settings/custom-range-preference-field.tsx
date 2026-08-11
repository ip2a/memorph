import { useEffect, useState } from "react";
import { Field, FieldContent, FieldDescription, FieldTitle } from "@/components/ui/field";
import {
  countCustomRangeDays,
  defaultCustomRangePreference,
  formatCustomRangeLabel,
  readCustomRangePreference,
  writeCustomRangePreference,
} from "@/lib/custom-range-preferences";
import { useI18n } from "@/lib/i18n-context";

export function CustomRangePreferenceField() {
  const { t } = useI18n();
  const [range, setRange] = useState(defaultCustomRangePreference);

  useEffect(() => {
    setRange(readCustomRangePreference() ?? defaultCustomRangePreference());
  }, []);

  function update(next: Partial<{ from: string; to: string }>) {
    setRange((current) => {
      const value = { ...current, ...next };
      writeCustomRangePreference(value);
      return value;
    });
  }

  const dayCount = countCustomRangeDays(range.from, range.to);
  const rangeLabel = formatCustomRangeLabel(range.from, range.to);

  return (
    <div
      className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start"
      data-custom-range-preference
    >
      <Field orientation="vertical">
        <FieldContent>
          <FieldTitle>{t("customRangePreference")}</FieldTitle>
          <FieldDescription>{t("customRangePreferenceHint")}</FieldDescription>
        </FieldContent>
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <input
            aria-label={t("customRangeStartDate")}
            className="border-input bg-background h-9 rounded-md border px-2"
            type="date"
            value={range.from}
            onChange={(event) => update({ from: event.target.value })}
          />
          <span>{t("customRangeTo")}</span>
          <input
            aria-label={t("customRangeEndDate")}
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
        <span className="text-xs text-muted-foreground">{t("customRangePreview")}</span>
        <span className="font-medium tabular-nums">{rangeLabel}</span>
        <span className="text-muted-foreground tabular-nums">
          {dayCount > 0 ? t("skillsDays", { count: dayCount }) : t("customRangeInvalid")}
        </span>
      </div>
    </div>
  );
}
