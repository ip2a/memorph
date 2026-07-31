export type SkillsStatsCustomRangePreference = {
  from: string;
  to: string;
};

const STORAGE_KEY = "memorph.skillsStatsCustomRange";

export function readSkillsStatsCustomRange(): SkillsStatsCustomRangePreference | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<SkillsStatsCustomRangePreference>;
    if (!parsed.from || !parsed.to) return null;
    return { from: parsed.from, to: parsed.to };
  } catch {
    return null;
  }
}

export function writeSkillsStatsCustomRange(
  range: SkillsStatsCustomRangePreference,
): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(range));
}

export function formatSkillsStatsRangeLabel(from: string, to: string) {
  const format = (value: string) => {
    const [year, month, day] = value.split("-");
    if (!year || !month || !day) return value;
    return `${Number(month)}/${Number(day)}`;
  };
  return `${format(from)}–${format(to)}`;
}

export function countSkillsStatsRangeDays(from: string, to: string) {
  const start = Date.parse(`${from}T00:00:00`);
  const end = Date.parse(`${to}T00:00:00`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return 0;
  return Math.floor((end - start) / 86_400_000) + 1;
}
