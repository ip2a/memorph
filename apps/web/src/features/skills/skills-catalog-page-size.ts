export const SKILLS_CATALOG_PAGE_SIZE_PRESETS = [10, 20, 30, 50] as const;
export const DEFAULT_SKILLS_CATALOG_PAGE_SIZE = 20;
export const MIN_SKILLS_CATALOG_PAGE_SIZE = 1;
export const MAX_SKILLS_CATALOG_PAGE_SIZE = 200;

export type SkillsCatalogPageSizePreset =
  (typeof SKILLS_CATALOG_PAGE_SIZE_PRESETS)[number];

export function clampSkillsCatalogPageSize(value?: number | null): number {
  const parsed = Number(value ?? DEFAULT_SKILLS_CATALOG_PAGE_SIZE);
  if (!Number.isFinite(parsed)) return DEFAULT_SKILLS_CATALOG_PAGE_SIZE;
  return Math.max(
    MIN_SKILLS_CATALOG_PAGE_SIZE,
    Math.min(MAX_SKILLS_CATALOG_PAGE_SIZE, Math.round(parsed)),
  );
}

export function isSkillsCatalogPageSizePreset(
  value: number,
): value is SkillsCatalogPageSizePreset {
  return SKILLS_CATALOG_PAGE_SIZE_PRESETS.includes(
    value as SkillsCatalogPageSizePreset,
  );
}
