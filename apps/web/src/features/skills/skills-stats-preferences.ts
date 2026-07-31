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
