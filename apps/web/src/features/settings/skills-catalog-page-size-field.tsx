import { Field, FieldContent, FieldDescription, FieldTitle } from "@/components/ui/field";
import { SkillsCatalogPageSizeControls } from "@/features/skills/skills-catalog-page-size-controls";
import { clampSkillsCatalogPageSize } from "@/features/skills/skills-catalog-page-size";
import { useI18n } from "@/lib/i18n-context";

export function SkillsCatalogPageSizeField({
  value,
  onChange,
}: {
  value: number;
  onChange: (next: number) => void;
}) {
  const { t } = useI18n();
  const clamped = clampSkillsCatalogPageSize(value);

  return (
    <Field orientation="responsive">
      <FieldContent>
        <FieldTitle>{t("skillsCatalogPageSizePreference")}</FieldTitle>
        <FieldDescription>
          {t("skillsCatalogPageSizePreferenceHint")}
        </FieldDescription>
      </FieldContent>
      <SkillsCatalogPageSizeControls value={clamped} onChange={onChange} />
    </Field>
  );
}
