import { useEffect, useState } from "react";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldTitle } from "@/components/ui/field";
import {
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

  return (
    <FieldGroup>
      <Field orientation="responsive">
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
    </FieldGroup>
  );
}
