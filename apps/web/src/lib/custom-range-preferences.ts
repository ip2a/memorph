import type { StatsDashboardRange } from "@/lib/types";

export type CustomRangePreference = {
  from: string;
  to: string;
};

const STORAGE_KEY = "memorph.customStatsRange";
const LEGACY_STORAGE_KEY = "memorph.skillsStatsCustomRange";

export function defaultCustomRangePreference(): CustomRangePreference {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - 29);
  return {
    from: localDate(from),
    to: localDate(to),
  };
}

function localDate(date: Date) {
  const offset = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offset).toISOString().slice(0, 10);
}

export function readCustomRangePreference(): CustomRangePreference | null {
  try {
    let raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      raw = localStorage.getItem(LEGACY_STORAGE_KEY);
      if (raw) {
        localStorage.setItem(STORAGE_KEY, raw);
        localStorage.removeItem(LEGACY_STORAGE_KEY);
      }
    }
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<CustomRangePreference>;
    if (!parsed.from || !parsed.to) return null;
    return { from: parsed.from, to: parsed.to };
  } catch {
    return null;
  }
}

export function writeCustomRangePreference(range: CustomRangePreference): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(range));
}

export function readCustomRangePreferenceOrDefault(): CustomRangePreference {
  return readCustomRangePreference() ?? defaultCustomRangePreference();
}

export function formatCustomRangeLabel(from: string, to: string) {
  const format = (value: string) => {
    const [year, month, day] = value.split("-");
    if (!year || !month || !day) return value;
    return `${Number(month)}/${Number(day)}`;
  };
  return `${format(from)}–${format(to)}`;
}

export function countCustomRangeDays(from: string, to: string) {
  const start = Date.parse(`${from}T00:00:00`);
  const end = Date.parse(`${to}T00:00:00`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return 0;
  return Math.floor((end - start) / 86_400_000) + 1;
}

export function resolvePreferredStatsDashboardRange(
  preference: CustomRangePreference = readCustomRangePreferenceOrDefault(),
): StatsDashboardRange {
  const days = countCustomRangeDays(preference.from, preference.to);
  if (days <= 7) return "7d";
  if (days <= 30) return "30d";
  if (days <= 90) return "90d";
  return "all";
}
