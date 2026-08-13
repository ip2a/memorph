import { useEffect, useState } from "react";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldTitle } from "@/components/ui/field";
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
  const statusValue =
    dayCount > 0 ? `${rangeLabel} · ${t("skillsDays", { count: dayCount })}` : t("customRangeInvalid");

  return (
    <FieldGroup data-custom-range-preference>
      <Field orientation="responsive">
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
          <span className="text-muted-foreground">{t("customRangeTo")}</span>
          <input
            aria-label={t("customRangeEndDate")}
            className="border-input bg-background h-9 rounded-md border px-2"
            type="date"
            value={range.to}
            onChange={(event) => update({ to: event.target.value })}
          />
        </div>
      </Field>
      <Field orientation="responsive">
        <FieldContent>
          <FieldTitle>{t("customRangePreview")}</FieldTitle>
        </FieldContent>
        <span className="text-sm tabular-nums text-muted-foreground" aria-live="polite">
          {statusValue}
        </span>
      </Field>
    </FieldGroup>
  );
}
