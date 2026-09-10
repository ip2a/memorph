import { Input } from "@/components/ui/input";
import { Toggle } from "@/components/ui/toggle";
import {
  clampSkillsCatalogPageSize,
  SKILLS_CATALOG_PAGE_SIZE_PRESETS,
} from "@/features/skills/skills-catalog-page-size";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";

export function SkillsCatalogPageSizeControls({
  value,
  onChange,
}: {
  value: number;
  onChange: (next: number) => void;
}) {
  const { t } = useI18n();
  const clamped = clampSkillsCatalogPageSize(value);

  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="flex flex-wrap gap-2">
        {SKILLS_CATALOG_PAGE_SIZE_PRESETS.map((preset) => {
          const active = clamped === preset;
          return (
            <Toggle
              key={preset}
              pressed={active}
              variant="outline"
              size="sm"
              className={cn(
                "min-w-10 font-mono tabular-nums",
                active && "border-primary/50 bg-primary/10",
              )}
              aria-label={t("skillsCatalogPageSizePreset", { count: preset })}
              onPressedChange={() => onChange(preset)}
            >
              {preset}
            </Toggle>
          );
        })}
      </div>
      <Input
        className="w-32 font-mono tabular-nums"
        type="number"
        min={1}
        max={200}
        inputMode="numeric"
        value={String(clamped)}
        aria-label={t("skillsCatalogPageSizeCustom")}
        onChange={(event) =>
          onChange(clampSkillsCatalogPageSize(Number(event.target.value)))
        }
      />
    </div>
  );
}
