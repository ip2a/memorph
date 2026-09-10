import type { I18nKey } from "@/lib/i18n-core";

export function skillUsedByLabel(
  usedBy: string,
  t: (key: I18nKey, vars?: Record<string, string | number>) => string,
) {
  return usedBy === "all" ? t("skillsGlobalScope") : usedBy;
}
